#!/usr/bin/env bash
# Runs the Java fixture harness: builds it when needed, then `java harness.Main "$@"` with a pinned
# time zone, locale and encoding. stdout carries only protocol lines and JSON.
#
# usage: tools/java_fixture_harness/run.sh <descriptors|generate|verify|dump|roundtrip|selftest> [args]
# exit:  0 ok, 1 verification failure, 2 usage, 3 internal error, 4 missing JDK or Java sources

set -uo pipefail

HARNESS="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)"
REPO="$(cd "$HARNESS/../.." && pwd -P)"
REQUIRED_MAJOR=25

# The JDK derives sun.jnu.encoding (file names) from the locale and ignores -Dsun.jnu.encoding,
# so non-ASCII paths need a UTF-8 locale whatever the host's LANG is.
utf8_locale() {
    local available candidate
    available="$(locale -a 2>/dev/null | tr '[:upper:]' '[:lower:]' | tr -d '-')"
    for candidate in C.UTF-8 en_US.UTF-8; do
        if grep -qxF "$(tr '[:upper:]' '[:lower:]' <<<"$candidate" | tr -d '-')" <<<"$available"; then
            echo "$candidate"
            return
        fi
    done
    echo C.UTF-8
}
LC_ALL="$(utf8_locale)"
LANG="$LC_ALL"
export LC_ALL LANG
unset LANGUAGE

# shellcheck source=../../scripts/java_env.sh
if ! source "$REPO/scripts/java_env.sh" 2>/dev/null; then
    echo "java_fixture_harness: no JDK $REQUIRED_MAJOR at ${A3S_JAVA_HOME:-the java_env.sh default} (set A3S_JAVA_HOME)" >&2
    exit 4
fi
java_version="$(java --version 2>/dev/null | head -n 1)"
if [[ "$(awk '{print $2}' <<<"$java_version" | cut -d. -f1)" != "$REQUIRED_MAJOR" ]]; then
    echo "java_fixture_harness: JDK $REQUIRED_MAJOR required, found '$java_version' (set A3S_JAVA_HOME)" >&2
    exit 4
fi

# The build is incremental: it only prints "up to date" when nothing changed.
"$HARNESS/build.sh" >&2
status=$?
if [[ $status -ne 0 ]]; then
    exit "$status"
fi

FLAGS=(
    -Duser.timezone=UTC -Duser.language=en -Duser.country=US
    -Dfile.encoding=UTF-8 -Dsun.jnu.encoding=UTF-8 -Djava.awt.headless=true
    "-Da3s.harness.home=$HARNESS"
    "-Da3s.harness.javaCommit=$(cat "$HARNESS/build/java_commit" 2>/dev/null || echo unknown)"
)
if [[ -n "${TMPDIR:-}" ]]; then
    FLAGS+=("-Djava.io.tmpdir=$TMPDIR")
fi

exec java "${FLAGS[@]}" -cp "$HARNESS/build/classes" harness.Main "$@"
