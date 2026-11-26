# 通过同一 SSH 会话接收一次性 sudo 口令和面板自带的二进制。
set -eu
export PATH=/usr/sbin:/usr/bin:/sbin:/bin
umask 077
reader=$(id -un)
revision=$(deployment_identity)
[ "${revision%% *}" = "$1" ] || exit 41
upload=$(mktemp)
trap 'rm -f -- "$upload"' EXIT
trap 'exit 1' HUP INT TERM
printf 'READY %s\n' "$1"
IFS= read -r secret || exit 42
cat > "$upload"
digest=$(sha256sum "$upload")
[ "${digest%% *}" = "$2" ] || exit 42
if [ "$(id -u)" = 0 ]; then
    /bin/sh -c "$3" installer "$upload" "$2" "$reader" "$4" "$5" "$6"
elif [ -x /usr/bin/sudo ]; then
    if [ -n "$secret" ]; then
        printf '%s\n' "$secret" | /usr/bin/sudo -S -p '' -- /bin/sh -c "$3" installer "$upload" "$2" "$reader" "$4" "$5" "$6"
    else
        /usr/bin/sudo -n -- /bin/sh -c "$3" installer "$upload" "$2" "$reader" "$4" "$5" "$6"
    fi
else
    exit 43
fi
