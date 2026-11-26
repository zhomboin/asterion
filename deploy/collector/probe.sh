# 只读检查。部署目标绑定当前启动实例与实际 SSH 账号。
set -eu
export PATH=/usr/sbin:/usr/bin:/sbin:/bin
system=$(uname -s)
arch=$(uname -m)
enabled=no
state=unsupported
if [ "$system" = Linux ] && [ -d /run/systemd/system ] && command -v systemd-sysusers >/dev/null; then
    systemctl is-enabled --quiet asterion-collector.service && enabled=yes
    if systemctl is-active --quiet asterion-collector.service; then
        state=active
    elif [ -e /usr/local/bin/asterion-collector ] || systemctl cat asterion-collector.service >/dev/null 2>&1; then
        state=inactive
    else
        state=missing
    fi
fi
revision=-
if [ "$state" != unsupported ]; then
    revision=$(deployment_identity)
    revision=${revision%% *}
fi
printf '%s\n' "$arch" "$state" "$revision" "$(id -u)" "$enabled"

config=/etc/default/asterion-collector
if [ -r "$config" ]; then
    awk -F= '
        BEGIN { values[1]=15; values[2]=7; values[3]=256 }
        {
            key=$1; gsub(/[[:space:]]/, "", key)
            i=0
            if (key=="ASTERION_COLLECTOR_INTERVAL_SECS") i=1
            if (key=="ASTERION_COLLECTOR_RETENTION_DAYS") i=2
            if (key=="ASTERION_COLLECTOR_MAX_MIB") i=3
            if (i) {
                value=$2; gsub(/^[[:space:]]+|[[:space:]]+$/, "", value)
                gsub(/^"|"$/, "", value)
                values[i]=(NF==2 && value ~ /^[0-9]+$/ && length(value)<9) ? value : "-"
            }
        }
        END { for (i=1; i<=3; i++) print values[i] }
    ' "$config"
elif [ -e "$config" ]; then
    printf '%s\n' - - -
else
    printf '%s\n' 15 7 256
fi
