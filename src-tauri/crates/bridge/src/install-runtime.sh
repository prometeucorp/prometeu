set -eu
umask 077
base=$1
digest=$2
for directory in "$base" "$base/runtime" "$base/runtime/$digest"; do
    test ! -L "$directory" || { printf 'Runtime directory is a symbolic link' >&2; exit 1; }
    mkdir -p -- "$directory"
done
temporary=$(mktemp "$base/runtime/$digest/.install.XXXXXX")
trap 'rm -f -- "$temporary"' EXIT HUP INT TERM
cat > "$temporary"
actual=$(sha256sum "$temporary")
test "${actual%% *}" = "$digest" || { printf 'Runtime package checksum mismatch' >&2; exit 1; }
chmod 700 "$temporary"
"$temporary" --help > /dev/null
executable="$base/runtime/$digest/prometeu-runtime"
mv -f -- "$temporary" "$executable"
printf '%s\n' "$executable"
