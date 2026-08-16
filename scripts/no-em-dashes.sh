#!/usr/bin/env bash
# Rule 1 in CLAUDE.md. The pattern is built from raw bytes so this file itself holds none,
# and so the check does not depend on the locale being UTF-8.
set -u

status=0
grep -rIn --exclude-dir=.git --exclude-dir=target $'\xe2\x80\x94' . || status=$?

case "$status" in
  0) echo "Em dashes found. See rule 1 in CLAUDE.md." >&2; exit 1 ;;
  1) exit 0 ;;
  *) echo "The em dash check could not run." >&2; exit 1 ;;
esac
