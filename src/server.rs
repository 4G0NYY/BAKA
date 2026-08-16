//! The watch, serve and files modes: BAKA with no terminal attached. They read the
//! same settings the interface does and run the same queue underneath.

use std::ffi::OsString;
use std::fs;
use std::future::Future;
use std::io::SeekFrom;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use tokio::io::{
    AsyncBufRead, AsyncBufReadExt, AsyncReadExt, AsyncSeekExt, AsyncWrite, AsyncWriteExt, BufReader,
};
use tokio::net::{TcpListener, TcpStream};

use crate::config::Settings;
use crate::engine::{Engine, Input};
use crate::search::{encode, human_size};

/// How often the watched folder is looked at, and how long a file has to have been
/// sitting still before it is read.
const LOOK_EVERY: Duration = Duration::from_secs(2);

/// What a handled file is renamed to, so the next look leaves it alone.
const TAKEN: &str = "taken";
const FAILED: &str = "failed";

/// A magnet link is a few hundred bytes. Anything past this is not one.
const MOST_A_REQUEST_MAY_SEND: usize = 64 * 1024;

/// Download anything dropped into a folder.
pub async fn watch(settings: &Settings, folder: &Path) -> Result<()> {
    fs::create_dir_all(folder).with_context(|| format!("could not open {}", folder.display()))?;
    let engine = Arc::new(Engine::start(settings).await?);
    println!(
        "Watching {}. Drop a magnet link, an infohash or a torrent file in it.",
        folder.display()
    );
    let looking = look(&engine, settings, folder);
    until_stopped(settings, &engine, looking).await
}

/// Accept magnets over HTTP.
pub async fn serve(settings: &Settings) -> Result<()> {
    let engine = Arc::new(Engine::start(settings).await?);
    let at = SocketAddr::new(settings.server.bind, settings.server.intake_port);
    let listener = open(at).await?;
    println!("Magnet intake on http://{at}");

    let taking = async {
        loop {
            let (mut stream, _) = listener.accept().await?;
            let engine = Arc::clone(&engine);
            let folder = settings.downloads.folder.clone();
            tokio::spawn(async move {
                if let Err(e) = intake(&mut stream, &engine, &folder).await {
                    eprintln!("{e:#}");
                }
            });
        }
    };
    until_stopped(settings, &engine, taking).await
}

/// Serve finished downloads over HTTP.
pub async fn files(settings: &Settings) -> Result<()> {
    let root = settings.downloads.folder.clone();
    fs::create_dir_all(&root).with_context(|| format!("could not open {}", root.display()))?;
    let engine = Engine::start(settings).await?;
    let at = SocketAddr::new(settings.server.bind, settings.server.files_port);
    let listener = open(at).await?;
    println!("Serving {} on http://{at}", root.display());

    let serving = async {
        loop {
            let (mut stream, _) = listener.accept().await?;
            let root = root.clone();
            tokio::spawn(async move {
                if let Err(e) = deliver(&mut stream, &root).await {
                    eprintln!("{e:#}");
                }
            });
        }
    };
    until_stopped(settings, &engine, serving).await
}

/// Detach and keep running after a logout. The child is this same command without the
/// flag that asked for it, and it writes nowhere, so nothing waits on a console.
pub fn detach() -> Result<()> {
    let mut command = Command::new(std::env::current_exe()?);
    command
        .args(
            std::env::args_os()
                .skip(1)
                .filter(|argument| argument != &OsString::from("--daemon")),
        )
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    loosen(&mut command);

    let child = command
        .spawn()
        .context("could not start the background run")?;
    println!("Running in the background as process {}.", child.id());
    Ok(())
}

#[cfg(windows)]
fn loosen(command: &mut Command) {
    use std::os::windows::process::CommandExt;

    // No console of its own, and its own group, so closing this window and Ctrl+C in
    // it both leave the background run alone.
    const DETACHED_PROCESS: u32 = 0x0000_0008;
    const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
    command.creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP);
}

#[cfg(unix)]
fn loosen(command: &mut Command) {
    use std::os::unix::process::CommandExt;

    // Its own process group, so a hangup meant for this terminal is not meant for it.
    command.process_group(0);
}

/// Every headless mode is the same shape: the job, the queue running underneath it,
/// and Ctrl+C meaning stop rather than kill.
async fn until_stopped<J>(settings: &Settings, engine: &Engine, job: J) -> Result<()>
where
    J: Future<Output = Result<()>>,
{
    let outcome = tokio::select! {
        result = job => result,
        result = queue(settings, engine) => result,
        _ = tokio::signal::ctrl_c() => {
            println!("Stopping.");
            Ok(())
        }
    };
    engine.shutdown().await;
    outcome
}

/// The same queue the interface runs, so the concurrency limits and stop at ratio
/// mean the same thing with nobody watching.
async fn queue(settings: &Settings, engine: &Engine) -> Result<()> {
    loop {
        engine.enforce(settings).await?;
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
}

async fn open(at: SocketAddr) -> Result<TcpListener> {
    TcpListener::bind(at)
        .await
        .with_context(|| format!("could not listen on {at}"))
}

async fn look(engine: &Arc<Engine>, settings: &Settings, folder: &Path) -> Result<()> {
    loop {
        for file in settled(folder)? {
            let outcome = take(engine, settings, &file);
            let (mark, told) = match &outcome {
                Ok(count) => (TAKEN, format!("{count} added from {}", named(&file))),
                Err(e) => (FAILED, format!("{}: {e:#}", named(&file))),
            };
            match outcome.is_ok() {
                true => println!("{told}"),
                false => eprintln!("{told}"),
            }
            fs::rename(&file, marked(&file, mark))
                .with_context(|| format!("could not rename {}", file.display()))?;
        }
        tokio::time::sleep(LOOK_EVERY).await;
    }
}

fn take(engine: &Arc<Engine>, settings: &Settings, file: &Path) -> Result<usize> {
    let offered = offered(file)?;
    if offered.is_empty() {
        bail!("nothing in it names a torrent");
    }
    hand_over(&offered, engine, &settings.downloads.folder);
    Ok(offered.len())
}

/// Adding a magnet waits for peers to hand over the file list, and that can take
/// longer than whatever asked for it is willing to wait. A folder with ten things in
/// it would work through them one at a time, and an HTTP client would sit on an open
/// request for minutes, so every add gets a task and the answer comes now.
fn hand_over(offered: &[Input], engine: &Arc<Engine>, folder: &Path) {
    for input in offered {
        let engine = Arc::clone(engine);
        let input = input.clone();
        let folder = folder.to_path_buf();
        tokio::spawn(async move {
            if let Err(e) = engine.add(&input, &folder).await {
                eprintln!("{e:#}");
            }
        });
    }
}

/// A torrent file is the torrent. Anything else is read as text, and every line of it
/// that names something downloadable is taken.
fn offered(file: &Path) -> Result<Vec<Input>> {
    if file
        .extension()
        .is_some_and(|kind| kind.eq_ignore_ascii_case("torrent"))
    {
        return Ok(vec![Input::File(file.to_path_buf())]);
    }
    let text =
        fs::read_to_string(file).with_context(|| format!("could not read {}", file.display()))?;
    Ok(listed(&text))
}

fn listed(text: &str) -> Vec<Input> {
    text.lines()
        .filter_map(|line| Input::parse(line).ok())
        .collect()
}

/// A file still being copied in is not ready to be read, and how long ago it was
/// written is the only signal there is without watching the filesystem itself.
fn settled(folder: &Path) -> Result<Vec<PathBuf>> {
    let mut ready = Vec::new();
    for entry in
        fs::read_dir(folder).with_context(|| format!("could not read {}", folder.display()))?
    {
        let entry = entry?;
        let file = entry.path();
        let facts = entry.metadata()?;
        if !facts.is_file() || already_handled(&file) {
            continue;
        }
        let still = facts
            .modified()
            .ok()
            .and_then(|written| written.elapsed().ok())
            .is_some_and(|ago| ago >= LOOK_EVERY);
        if still {
            ready.push(file);
        }
    }
    ready.sort();
    Ok(ready)
}

fn marked(file: &Path, outcome: &str) -> PathBuf {
    let mut name = file.as_os_str().to_os_string();
    name.push(".");
    name.push(outcome);
    PathBuf::from(name)
}

fn already_handled(file: &Path) -> bool {
    file.extension()
        .is_some_and(|kind| kind == TAKEN || kind == FAILED)
}

fn named(file: &Path) -> String {
    file.file_name().unwrap_or_default().display().to_string()
}

async fn intake(stream: &mut TcpStream, engine: &Arc<Engine>, folder: &Path) -> Result<()> {
    let (reader, mut writer) = stream.split();
    let request = read(&mut BufReader::new(reader)).await?;

    let (status, said) = match request.method.as_str() {
        "GET" => ("200 OK", running(engine)),
        "POST" => ("200 OK", accept(&request.body, engine, folder)),
        _ => (
            "405 Method Not Allowed",
            "Send a magnet with POST.\n".into(),
        ),
    };
    reply(&mut writer, status, "text/plain; charset=utf-8", &said).await
}

/// Adding a magnet waits for peers to hand over the file list, which can take longer
/// than a request should, so what was understood is answered now and added after.
fn accept(body: &str, engine: &Arc<Engine>, folder: &Path) -> String {
    let offered = listed(body);
    if offered.is_empty() {
        return "Nothing there names a torrent.\n".to_string();
    }

    for input in &offered {
        let engine = Arc::clone(engine);
        let input = input.clone();
        let folder = folder.to_path_buf();
        tokio::spawn(async move {
            if let Err(e) = engine.add(&input, &folder).await {
                eprintln!("{e:#}");
            }
        });
    }
    format!("{} taken.\n", offered.len())
}

fn running(engine: &Engine) -> String {
    let torrents = engine.snapshot();
    if torrents.is_empty() {
        return "Nothing here yet.\n".to_string();
    }

    let mut said = String::new();
    for torrent in torrents {
        let share = match torrent.total_bytes {
            0 => 0.0,
            total => torrent.done_bytes as f64 / total as f64 * 100.0,
        };
        said.push_str(&format!(
            "{:>5.1}%  {:<9}  {}\n",
            share,
            torrent.state.to_string(),
            torrent.name.unwrap_or(torrent.info_hash),
        ));
    }
    said
}

async fn deliver(stream: &mut TcpStream, root: &Path) -> Result<()> {
    let (reader, mut writer) = stream.split();
    let request = read(&mut BufReader::new(reader)).await?;

    if request.method != "GET" {
        return reply(
            &mut writer,
            "405 Method Not Allowed",
            "text/plain; charset=utf-8",
            "This one only serves files.\n",
        )
        .await;
    }

    let Some(target) = inside(root, &request.path) else {
        return reply(
            &mut writer,
            "404 Not Found",
            "text/plain; charset=utf-8",
            "No such file.\n",
        )
        .await;
    };

    if target.is_dir() {
        let page = listing(&target, &request.path)?;
        return reply(&mut writer, "200 OK", "text/html; charset=utf-8", &page).await;
    }
    send(&mut writer, &target, request.range).await
}

/// A request path is joined to the download folder and to nothing else. Anything that
/// climbs out of it is refused rather than quietly corrected.
fn inside(root: &Path, path: &str) -> Option<PathBuf> {
    let mut target = root.to_path_buf();
    for part in path.split('/').filter(|part| !part.is_empty()) {
        if part == "." || part == ".." || part.contains('\\') || part.contains(':') {
            return None;
        }
        target.push(part);
    }

    // A link inside the folder can still point outside it, and only the real path
    // both sides resolve to says so.
    let target = target.canonicalize().ok()?;
    let root = root.canonicalize().ok()?;
    target.starts_with(root).then_some(target)
}

fn listing(target: &Path, path: &str) -> Result<String> {
    let mut entries: Vec<fs::DirEntry> = fs::read_dir(target)
        .with_context(|| format!("could not read {}", target.display()))?
        .collect::<Result<_, _>>()?;
    entries.sort_by_key(fs::DirEntry::file_name);

    // A folder called "A Folder" is a link only once its space is written the way a
    // URL writes one, and the same goes for every folder above it.
    let parts: Vec<&str> = path.split('/').filter(|part| !part.is_empty()).collect();
    let here = linked(&parts);
    let up = linked(&parts[..parts.len().saturating_sub(1)]);

    let mut page = format!(
        "<!doctype html>\n<title>BAKA files</title>\n<h1>{}</h1>\n<ul>\n",
        escaped(match parts.is_empty() {
            true => "/",
            false => path,
        })
    );
    if !parts.is_empty() {
        page.push_str(&format!("<li><a href=\"{up}/\">..</a></li>\n"));
    }

    for entry in entries {
        let name = entry.file_name().display().to_string();
        let folder = entry.file_type().is_ok_and(|kind| kind.is_dir());
        let size = match folder {
            true => String::new(),
            false => format!(" ({})", human_size(entry.metadata().map_or(0, |f| f.len()))),
        };
        page.push_str(&format!(
            "<li><a href=\"{here}/{}{}\">{}{}</a>{size}</li>\n",
            encode(&name),
            if folder { "/" } else { "" },
            escaped(&name),
            if folder { "/" } else { "" },
        ));
    }
    page.push_str("</ul>\n");
    Ok(page)
}

fn linked(parts: &[&str]) -> String {
    parts
        .iter()
        .map(|part| format!("/{}", encode(part)))
        .collect()
}

/// A player asks for the part of a film it is about to show, and without an answer to
/// that it can only start from the beginning.
async fn send<W: AsyncWrite + Unpin>(
    writer: &mut W,
    target: &Path,
    range: Option<(u64, Option<u64>)>,
) -> Result<()> {
    let mut file = tokio::fs::File::open(target)
        .await
        .with_context(|| format!("could not open {}", target.display()))?;
    let size = file.metadata().await?.len();
    let kind = content_type(target);

    let Some(last) = size.checked_sub(1) else {
        return reply(writer, "200 OK", kind, "").await;
    };
    let (from, to) = match range {
        None => (0, last),
        Some((from, until)) => (from.min(last), until.unwrap_or(last).min(last)),
    };
    if from > to {
        return reply(
            writer,
            "416 Range Not Satisfiable",
            "text/plain; charset=utf-8",
            "That part of the file is not there.\n",
        )
        .await;
    }

    let head = match range.is_some() {
        true => {
            format!("HTTP/1.1 206 Partial Content\r\nContent-Range: bytes {from}-{to}/{size}\r\n")
        }
        false => "HTTP/1.1 200 OK\r\n".to_string(),
    };
    writer
        .write_all(
            format!(
                "{head}Content-Type: {kind}\r\nContent-Length: {}\r\nAccept-Ranges: bytes\r\nConnection: close\r\n\r\n",
                to - from + 1
            )
            .as_bytes(),
        )
        .await?;

    file.seek(SeekFrom::Start(from)).await?;
    let mut left = to - from + 1;
    let mut chunk = vec![0; 64 * 1024];
    while left > 0 {
        let wanted = chunk.len().min(left as usize);
        let read = file.read(&mut chunk[..wanted]).await?;
        if read == 0 {
            break;
        }
        writer.write_all(&chunk[..read]).await?;
        left -= read as u64;
    }
    writer.flush().await?;
    Ok(())
}

fn content_type(target: &Path) -> &'static str {
    let extension = target
        .extension()
        .unwrap_or_default()
        .display()
        .to_string()
        .to_ascii_lowercase();
    match extension.as_str() {
        "mkv" => "video/x-matroska",
        "mp4" | "m4v" => "video/mp4",
        "webm" => "video/webm",
        "avi" => "video/x-msvideo",
        "mp3" => "audio/mpeg",
        "flac" => "audio/flac",
        "srt" | "txt" | "nfo" => "text/plain; charset=utf-8",
        "jpg" | "jpeg" => "image/jpeg",
        "png" => "image/png",
        // Everything else is offered as something to save rather than to open.
        _ => "application/octet-stream",
    }
}

struct Request {
    method: String,
    path: String,
    range: Option<(u64, Option<u64>)>,
    body: String,
}

async fn read<R: AsyncBufRead + Unpin>(reader: &mut R) -> Result<Request> {
    let mut line = String::new();
    reader.read_line(&mut line).await?;
    let mut parts = line.split_whitespace();
    let method = parts.next().unwrap_or_default().to_string();
    let target = parts.next().unwrap_or_default();
    let path = decode(target.split('?').next().unwrap_or_default());

    let mut length = 0;
    let mut range = None;
    loop {
        let mut header = String::new();
        if reader.read_line(&mut header).await? == 0 {
            break;
        }
        let header = header.trim_end();
        if header.is_empty() {
            break;
        }
        let Some((name, value)) = header.split_once(':') else {
            continue;
        };
        let value = value.trim();
        if name.eq_ignore_ascii_case("content-length") {
            length = value.parse().unwrap_or(0);
        }
        if name.eq_ignore_ascii_case("range") {
            range = wanted(value);
        }
    }

    let mut body = vec![0; length.min(MOST_A_REQUEST_MAY_SEND)];
    reader.read_exact(&mut body).await?;

    Ok(Request {
        method,
        path,
        range,
        body: String::from_utf8_lossy(&body).into_owned(),
    })
}

/// One range, which is what a player asks for. A request for several is answered in
/// full instead, which is allowed and is what a client that asks for one gets anyway.
fn wanted(header: &str) -> Option<(u64, Option<u64>)> {
    let (from, to) = header.trim().strip_prefix("bytes=")?.split_once('-')?;
    if to.contains(',') {
        return None;
    }
    Some((from.trim().parse().ok()?, to.trim().parse().ok()))
}

async fn reply<W: AsyncWrite + Unpin>(
    writer: &mut W,
    status: &str,
    kind: &str,
    body: &str,
) -> Result<()> {
    writer
        .write_all(
            format!(
                "HTTP/1.1 {status}\r\nContent-Type: {kind}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            )
            .as_bytes(),
        )
        .await?;
    writer.write_all(body.as_bytes()).await?;
    writer.flush().await?;
    Ok(())
}

fn decode(path: &str) -> String {
    let mut out = String::with_capacity(path.len());
    let mut rest = path;
    while let Some(at) = rest.find('%') {
        out.push_str(&rest[..at]);
        let escape = rest
            .get(at + 1..at + 3)
            .and_then(|hex| u8::from_str_radix(hex, 16).ok().map(|byte| byte as char));
        match escape {
            Some(letter) => {
                out.push(letter);
                rest = &rest[at + 3..];
            }
            None => {
                out.push('%');
                rest = &rest[at + 1..];
            }
        }
    }
    out.push_str(rest);
    out
}

fn escaped(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_request_gives_up_its_method_path_and_body() {
        let raw = "POST /add?x=1 HTTP/1.1\r\nHost: localhost\r\nContent-Length: 7\r\n\r\nmagnet:";
        let request = read(&mut BufReader::new(raw.as_bytes())).await.unwrap();
        assert_eq!(request.method, "POST");
        assert_eq!(request.path, "/add");
        assert_eq!(request.body, "magnet:");
        assert_eq!(request.range, None);
    }

    #[tokio::test]
    async fn a_path_arrives_decoded_so_a_file_with_a_space_can_be_asked_for() {
        let raw = "GET /Some%20Film/part%201.mkv HTTP/1.1\r\n\r\n";
        let request = read(&mut BufReader::new(raw.as_bytes())).await.unwrap();
        assert_eq!(request.path, "/Some Film/part 1.mkv");
    }

    #[tokio::test]
    async fn a_player_asking_for_part_of_a_film_is_understood() {
        let raw = "GET /film.mkv HTTP/1.1\r\nRange: bytes=200-999\r\n\r\n";
        let request = read(&mut BufReader::new(raw.as_bytes())).await.unwrap();
        assert_eq!(request.range, Some((200, Some(999))));
    }

    #[test]
    fn a_range_without_an_end_means_the_rest_of_the_file() {
        assert_eq!(wanted("bytes=500-"), Some((500, None)));
        assert_eq!(wanted("bytes=0-99"), Some((0, Some(99))));
        assert_eq!(wanted("lines=1-2"), None);
        assert_eq!(wanted("bytes=0-99,200-299"), None);
    }

    #[test]
    fn a_path_that_climbs_out_of_the_download_folder_is_refused() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        assert!(inside(root, "/../..").is_none());
        assert!(inside(root, "/src/../../etc").is_none());
        assert!(inside(root, "/C:/Windows").is_none());
        assert!(inside(root, "/nothing-here.txt").is_none());
    }

    #[test]
    fn a_path_inside_the_download_folder_is_served() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        assert!(inside(root, "/Cargo.toml").is_some());
        assert!(inside(root, "/src").is_some());
        assert_eq!(inside(root, "/"), root.canonicalize().ok());
    }

    #[test]
    fn a_dropped_file_gives_up_every_line_that_names_a_torrent() {
        let text = "magnet:?xt=urn:btih:cab507494d02ebb1178b38f2e9d7be299c86b862\n\
                    not a torrent\n\
                    CAB507494D02EBB1178B38F2E9D7BE299C86B863\n";
        assert_eq!(listed(text).len(), 2);
        assert!(listed("nothing here at all").is_empty());
    }

    #[test]
    fn a_handled_file_is_marked_so_the_next_look_leaves_it_alone() {
        let dropped = Path::new("watch/thing.torrent");
        let handled = marked(dropped, TAKEN);
        assert!(handled.ends_with("thing.torrent.taken"));
        assert!(already_handled(&handled));
        assert!(already_handled(&marked(dropped, FAILED)));
        assert!(!already_handled(dropped));
    }

    #[test]
    fn a_file_is_offered_as_the_kind_of_thing_its_name_says_it_is() {
        let torrent = Path::new("watch/debian.torrent");
        assert_eq!(
            offered(torrent).unwrap(),
            vec![Input::File(torrent.to_path_buf())]
        );
        assert!(offered(Path::new("watch/no-such-file.txt")).is_err());
    }

    #[test]
    fn what_a_browser_is_told_a_file_is_follows_its_name() {
        assert_eq!(content_type(Path::new("a/b/film.mkv")), "video/x-matroska");
        assert_eq!(
            content_type(Path::new("subs.SRT")),
            "text/plain; charset=utf-8"
        );
        assert_eq!(
            content_type(Path::new("game.iso")),
            "application/octet-stream"
        );
        assert_eq!(
            content_type(Path::new("no-extension")),
            "application/octet-stream"
        );
    }

    #[test]
    fn a_name_a_browser_would_read_as_markup_is_written_out_as_text() {
        assert_eq!(escaped("Tom & <Jerry>"), "Tom &amp; &lt;Jerry&gt;");
    }

    #[test]
    fn a_folder_with_a_space_in_its_name_is_still_a_link() {
        assert_eq!(linked(&["A Folder", "Season 1"]), "/A%20Folder/Season%201");
        assert_eq!(linked(&[]), "");
    }
}
