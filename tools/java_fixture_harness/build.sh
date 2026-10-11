#!/usr/bin/env bash
# Compiles the Java fixture harness: the minimal Arma3Sync closure, the commons-io stub, the
# harness sources and the historical variant classes. See README.md.
#
# usage: tools/java_fixture_harness/build.sh [--clean]
# exit:  0 ok, 3 compile or patch failure, 4 missing JDK or Java sources

set -uo pipefail

HARNESS="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)"
REPO="$(cd "$HARNESS/../.." && pwd -P)"
BUILD="$HARNESS/build"
REQUIRED_MAJOR=25

die() {
    local code="$1"
    shift
    echo "java_fixture_harness: $*" >&2
    exit "$code"
}

CLEAN=0
for arg in "$@"; do
    case "$arg" in
        --clean) CLEAN=1 ;;
        *) die 2 "unknown argument: $arg (usage: build.sh [--clean])" ;;
    esac
done

# javac reads source paths through sun.jnu.encoding, which follows the locale only.
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

# B2: JDK 25 from scripts/java_env.sh (A3S_JAVA_HOME overrides the default location).
# shellcheck source=../../scripts/java_env.sh
if ! source "$REPO/scripts/java_env.sh" 2>/dev/null; then
    die 4 "no JDK $REQUIRED_MAJOR at ${A3S_JAVA_HOME:-the java_env.sh default} (set A3S_JAVA_HOME)"
fi
javac_version="$(javac -version 2>/dev/null)" || die 4 "javac failed to start (set A3S_JAVA_HOME)"
java_version="$(java --version 2>/dev/null | head -n 1)" || die 4 "java failed to start (set A3S_JAVA_HOME)"
javac_major="$(awk '{print $2}' <<<"$javac_version" | cut -d. -f1)"
java_major="$(awk '{print $2}' <<<"$java_version" | cut -d. -f1)"
if [[ "$javac_major" != "$REQUIRED_MAJOR" || "$java_major" != "$REQUIRED_MAJOR" ]]; then
    die 4 "JDK $REQUIRED_MAJOR required, found javac '$javac_version' / java '$java_version' (set A3S_JAVA_HOME)"
fi

# B1: Java source root.
find_java_src() {
    if [[ -n "${A3S_JAVA_SRC:-}" ]]; then
        echo "$A3S_JAVA_SRC"
        return
    fi
    local candidate="$REPO/java_version/Arma3Sync/src/main/java"
    if [[ -d "$candidate/fr/soe/a3s" ]]; then
        echo "$candidate"
        return
    fi
    local common
    if common="$(git -C "$REPO" rev-parse --path-format=absolute --git-common-dir 2>/dev/null)"; then
        echo "$(dirname "$common")/java_version/Arma3Sync/src/main/java"
    fi
}
JAVA_SRC="$(find_java_src)"
if [[ -z "$JAVA_SRC" || ! -d "$JAVA_SRC/fr/soe/a3s" ]]; then
    die 4 "Java sources not found (set A3S_JAVA_SRC)"
fi
JAVA_SRC="$(cd "$JAVA_SRC" && pwd -P)"
A3S="$JAVA_SRC/fr/soe/a3s"
JAVA_CHECKOUT="$(cd "$JAVA_SRC/../../.." && pwd -P)"
JAVA_COMMIT="$(git -C "$JAVA_CHECKOUT" rev-parse --short HEAD 2>/dev/null || echo unknown)"

RBP_REL="fr/soe/a3s/dao/repository/RepositoryBuildProcessor.java"

# B6: historical variants. <name> <source relative to the Java root>.
VARIANTS=(
    "repository_pre_connections fr/soe/a3s/domain/repository/Repository.java"
    "prefs_pre_github fr/soe/a3s/domain/Preferences.java"
    "cfg_with_aia fr/soe/a3s/domain/configration/Configuration.java"
    "treeleaf_pre_workshop fr/soe/a3s/domain/TreeLeaf.java"
)

mkdir -p "$BUILD" || die 3 "cannot create $BUILD"
exec 9>"$BUILD/.lock" || die 3 "cannot open $BUILD/.lock"
flock 9 || die 3 "cannot lock $BUILD/.lock"

if [[ $CLEAN -eq 1 ]]; then
    find "$BUILD" -mindepth 1 -maxdepth 1 ! -name .lock -exec rm -rf {} + || die 3 "cannot clean $BUILD"
fi

# B7: the stamp covers every input of this script, including uncommitted Java changes (the
# implicitly compiled closure is only covered by the commit otherwise).
compute_stamp() {
    {
        (cd "$HARNESS" && find src stubs -type f -print0 | LC_ALL=C sort -z | xargs -0 sha256sum)
        sha256sum "$HARNESS/build.sh" | awk '{print $1}'
        (cd "$JAVA_SRC" && sha256sum "$RBP_REL")
        for entry in "${VARIANTS[@]}"; do
            (cd "$JAVA_SRC" && sha256sum "${entry#* }")
        done
        echo "$javac_version"
        echo "$JAVA_COMMIT"
        git -C "$JAVA_CHECKOUT" status --porcelain -- . 2>/dev/null
        git -C "$JAVA_CHECKOUT" diff HEAD -- . 2>/dev/null
    } | sha256sum | awk '{print $1}'
}
STAMP="$(compute_stamp)" || die 3 "cannot compute the build stamp"
if [[ -f "$BUILD/stamp" && "$(cat "$BUILD/stamp")" == "$STAMP" && -d "$BUILD/classes" ]]; then
    echo "up to date"
    exit 0
fi
rm -f "$BUILD/stamp" || die 3 "cannot remove the old stamp"

# apply_edit <file> <label> <expected count> <perl pattern> <perl replacement expression>
apply_edit() {
    local file="$1" label="$2" expected="$3" got
    got="$(PAT="$4" REP="$5" perl -0777 -i -pe '$n += s/$ENV{PAT}/$ENV{REP}/gee; END { print STDOUT ($n + 0) }' "$file")" ||
        die 3 "$label: perl failed on $file"
    if [[ "$got" != "$expected" ]]; then
        die 3 "$label: expected $expected got $got"
    fi
}

# B4: patched copy of RepositoryBuildProcessor (P1 pinned clock, P2 sorted listFiles).
rm -rf "$BUILD/patched-src" "$BUILD/variant-src" "$BUILD/classes.tmp" "$BUILD/variants.tmp" ||
    die 3 "cannot remove old build directories"
mkdir -p "$BUILD/patched-src/$(dirname "$RBP_REL")" || die 3 "cannot create patched-src"
PATCHED="$BUILD/patched-src/$RBP_REL"
cp "$JAVA_SRC/$RBP_REL" "$PATCHED" || die 3 "cannot copy $RBP_REL"
apply_edit "$PATCHED" "patch P1" 3 'new Date\(\)' '"harness.HarnessClock.now()"'
apply_edit "$PATCHED" "patch P2" 2 '\b([A-Za-z_][A-Za-z0-9_]*)\.listFiles\(\)' '"harness.HarnessFs.sorted($1.listFiles())"'

# B5: one javac call; everything not listed is pulled in through -sourcepath.
ARGS="$BUILD/javac.args"
{
    find "$HARNESS/src/harness" -name '*.java'
    find "$A3S/domain" "$A3S/constant" -name '*.java'
} | LC_ALL=C sort >"$ARGS" || die 3 "cannot write $ARGS"
mkdir -p "$BUILD/classes.tmp" || die 3 "cannot create classes.tmp"
echo "compiling the harness against $JAVA_SRC (Java commit $JAVA_COMMIT)"
if ! javac --release 25 -encoding UTF-8 -proc:none -implicit:class -nowarn -Xlint:none \
    -sourcepath "$BUILD/patched-src:$JAVA_SRC:$HARNESS/stubs:$HARNESS/src" \
    -d "$BUILD/classes.tmp" @"$ARGS" >&2; then
    die 3 "javac failed"
fi

# B6: variant classes, each compiled alone against the main classes.
mkdir -p "$BUILD/variants.tmp" || die 3 "cannot create variants.tmp"
for entry in "${VARIANTS[@]}"; do
    name="${entry%% *}"
    rel="${entry#* }"
    src="$BUILD/variant-src/$name/$rel"
    mkdir -p "$(dirname "$src")" || die 3 "cannot create the variant-src of $name"
    cp "$JAVA_SRC/$rel" "$src" || die 3 "cannot copy $rel"
    case "$name" in
        repository_pre_connections)
            apply_edit "$src" "variant $name" 2 \
                'private int (numberOfRepositoryCheckConnections|parallelUploadConnections)\b' \
                '"private transient int $1"'
            ;;
        prefs_pre_github)
            apply_edit "$src" "variant $name" 1 \
                'private Boolean preferGitHubUpdates;' '"private transient Boolean preferGitHubUpdates;"'
            ;;
        cfg_with_aia)
            apply_edit "$src" "variant $name" 1 \
                '([ \t]*)private Proxy proxy = new Proxy\(\);' \
                '"${1}private AiAOptions aiaOptions = new AiAOptions();\n${1}private Proxy proxy = new Proxy();"'
            ;;
        treeleaf_pre_workshop)
            apply_edit "$src" "variant $name" 2 \
                'private String (workshopId|dlcAppId);' '"private transient String $1;"'
            ;;
    esac
    out="$BUILD/variants.tmp/$name"
    mkdir -p "$out" || die 3 "cannot create $out"
    if ! javac --release 25 -encoding UTF-8 -proc:none -implicit:none -nowarn -Xlint:none \
        -cp "$BUILD/classes.tmp" -sourcepath "$BUILD/variant-src/$name" -d "$out" "$src" >&2; then
        die 3 "javac failed for variant $name"
    fi
    class_base="${rel%.java}"
    while IFS= read -r produced; do
        case "$produced" in
            "$class_base.class" | "$class_base\$"*.class) ;;
            *) die 3 "variant $name produced an unexpected class: $produced" ;;
        esac
    done < <(cd "$out" && find . -type f | sed 's|^\./||')
done

rm -rf "$BUILD/classes" "$BUILD/variants" || die 3 "cannot remove the old classes"
mv "$BUILD/classes.tmp" "$BUILD/classes" || die 3 "cannot install the classes"
mv "$BUILD/variants.tmp" "$BUILD/variants" || die 3 "cannot install the variants"
echo "$JAVA_COMMIT" >"$BUILD/java_commit" || die 3 "cannot write java_commit"
# The stamp is written last, so a failed build never looks up to date.
echo "$STAMP" >"$BUILD/stamp" || die 3 "cannot write the stamp"
echo "built $(find "$BUILD/classes" -name '*.class' | wc -l) classes into $BUILD/classes"
