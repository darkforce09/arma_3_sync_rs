#!/usr/bin/env bash
# Source this to get the JDK used by the Java fixture harness.
#   source scripts/java_env.sh
# Override with A3S_JAVA_HOME. The default is the user-local Temurin JDK 25 install
# (Bazzite/immutable distros: no system JDK).
export JAVA_HOME="${A3S_JAVA_HOME:-/var/home/Samuel/.local/share/jdk/jdk-25.0.4.1+1}"
export PATH="$JAVA_HOME/bin:$PATH"
# Silence cgroup v2 detection warnings inside the sandbox / flatpak scope.
export JAVA_TOOL_OPTIONS="${JAVA_TOOL_OPTIONS:--Xlog:disable -Xlog:all=error:stderr}"
if [[ ! -x "$JAVA_HOME/bin/javac" ]]; then
    echo "java_env.sh: no JDK at $JAVA_HOME (set A3S_JAVA_HOME)" >&2
    return 1 2>/dev/null || exit 1
fi
