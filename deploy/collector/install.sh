# 由面板通过已确认的部署任务调用。参数只包含已校验的摘要、读取账号和数字设置。
set -eu
export PATH=/usr/sbin:/usr/bin:/sbin:/bin
umask 077
source_file=$1
digest=$2
reader=$3
interval=$4
days=$5
mib=$6
unit=asterion-collector.service
binary=/usr/local/bin/asterion-collector
service=/etc/systemd/system/$unit
account=/etc/sysusers.d/asterion-collector.conf
config=/etc/default/asterion-collector
[ "$(id -u)" = 0 ] || exit 43
printf 'INSTALLING\n'
exec 9>/run/asterion-collector-deploy.lock
flock -n 9 || exit 44
stage=$(mktemp -d /usr/local/bin/.asterion-deploy.XXXXXXXX)
pending=
changed=0
granted=0
active=0
enabled=0
systemctl is-active --quiet "$unit" && active=1
systemctl is-enabled --quiet "$unit" && enabled=1

publish() {
    pending=$(mktemp -- "${2}.XXXXXXXX")
    cat -- "$1" > "$pending"
    if [ "$2" = "$config" ] && [ -f "$config" ]; then
        chown --reference="$config" "$pending"
        chmod --reference="$config" "$pending"
    else
        chmod "$3" "$pending"
    fi
    mv -fT -- "$pending" "$2"
    pending=
}

restore() {
    if [ -f "$stage/old-$1" ]; then
        pending=$(mktemp -- "${2}.XXXXXXXX")
        cp -p -- "$stage/old-$1" "$pending" && mv -fT -- "$pending" "$2"
    else
        rm -f -- "$2"
    fi
}

finish() {
    rc=$?
    trap - EXIT HUP INT TERM
    set +e
    [ -z "$pending" ] || rm -f -- "$pending"
    pending=
    if [ "$rc" != 0 ] && [ "$changed" = 1 ]; then
        [ "$enabled" = 1 ] || systemctl disable "$unit" >/dev/null 2>&1
        [ "$active" = 1 ] || systemctl stop "$unit" >/dev/null 2>&1
        [ "$granted" = 0 ] || gpasswd -d "$reader" asterion-collector >/dev/null
        restore binary "$binary"
        restore service "$service"
        restore account "$account"
        restore config "$config"
        systemctl daemon-reload
        [ "$active" = 0 ] || systemctl restart "$unit"
    fi
    [ -z "$pending" ] || rm -f -- "$pending"
    rm -f -- "$stage"/*
    rmdir -- "$stage"
    exit "$rc"
}
trap finish EXIT
trap 'exit 45' HUP INT TERM

# 仅提升已验证的面板产物。候选文件始终位于 root 私有目录。
head -c 134217729 -- "$source_file" > "$stage/binary"
actual=$(sha256sum "$stage/binary")
[ "${actual%% *}" = "$digest" ] || exit 42
chmod 0755 "$stage/binary"
"$stage/binary" --version >/dev/null || exit 46
for dir in /etc/sysusers.d /etc/default; do
    [ -d "$dir" ] || install -d -m 0755 "$dir"
done
save() {
    [ ! -L "$2" ] || exit 47
    if [ -e "$2" ]; then
        [ -f "$2" ] || exit 47
        cp -p -- "$2" "$stage/old-$1"
    fi
}
save binary "$binary"
save service "$service"
save account "$account"
save config "$config"
cat > "$stage/service" <<'ASTERION_UNIT'
@UNIT@
ASTERION_UNIT
cat > "$stage/account" <<'ASTERION_ACCOUNT'
@ACCOUNT@
ASTERION_ACCOUNT
cat > "$stage/config" <<'ASTERION_DEFAULTS'
@DEFAULTS@
ASTERION_DEFAULTS
if [ -n "$interval" ]; then
    if [ -f "$config" ]; then
        awk '!/^[[:space:]]*(ASTERION_COLLECTOR_INTERVAL_SECS|ASTERION_COLLECTOR_RETENTION_DAYS|ASTERION_COLLECTOR_MAX_MIB)[[:space:]]*=/' "$config" > "$stage/config"
    else
        : > "$stage/config"
    fi
    printf 'ASTERION_COLLECTOR_INTERVAL_SECS=%s\nASTERION_COLLECTOR_RETENTION_DAYS=%s\nASTERION_COLLECTOR_MAX_MIB=%s\n' "$interval" "$days" "$mib" >> "$stage/config"
fi
changed=1
publish "$stage/account" "$account" 0644
systemd-sysusers "$account" || exit 48
publish "$stage/service" "$service" 0644
if [ -n "$interval" ] || [ ! -f "$config" ]; then
    publish "$stage/config" "$config" 0644
fi
mv -fT -- "$stage/binary" "$binary"
systemctl daemon-reload && systemctl enable "$unit" || exit 48
# 原进程退出后跨过一秒，避免把现有缓存误认为此次启动产生的记录。
if [ "$active" = 1 ]; then
    systemctl stop "$unit" || exit 48
fi
sleep 1
stamp=$(date +%s)
systemctl start "$unit" || exit 48
systemctl is-active --quiet "$unit" && systemctl is-enabled --quiet "$unit" || exit 48
if [ "$reader" != root ] && ! id -nG "$reader" | tr " " "\n" | grep -Fxq asterion-collector; then
    usermod -aG asterion-collector -- "$reader" || exit 49
    granted=1
fi
attempt=0
while ! runuser -u "$reader" -- "$binary" ready "$stamp" >/dev/null 2>&1; do
    attempt=$((attempt + 1))
    [ "$attempt" -lt 15 ] || exit 50
    sleep 1
done
systemctl is-active --quiet "$unit" || exit 48
changed=0
printf '%s\n' "$stamp"
