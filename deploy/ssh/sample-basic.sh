# 无 Python 的 Linux 节点使用标准命令采集根分区和内核计数器。
set -eu
export LC_ALL=C
arch=$(uname -m)
case "$arch" in ''|*[!a-zA-Z0-9_-]*) exit 1 ;; esac
cpu_ticks() {
    awk '/^cpu / {for (i=2; i<=9; i++) total+=$i; printf "%.0f %.0f", total, $5+$6; exit}' /proc/stat
}
first=$(cpu_ticks)
sleep 1
second=$(cpu_ticks)
disk=$(df -Pk / | awk 'END {printf "%.0f %.0f", $2*1024, $4*1024}')
awk -v arch="$arch" -v stamp="$(date +%s)" -v first="$first" -v second="$second" -v disk="$disk" '
function capacity(total, free) {
    if (free>total) free=total
    return sprintf("{\"total_bytes\":%.0f,\"available_bytes\":%.0f,\"used_bytes\":%.0f}", total, free, total-free)
}
FILENAME=="/proc/stat" && /^cpu[0-9]+ / { cores++ }
FILENAME=="/proc/meminfo" { mem[$1]=$2*1024 }
FILENAME=="/proc/loadavg" { load=sprintf("[%f,%f,%f]", $1,$2,$3) }
FILENAME=="/proc/mounts" && $2=="/" && $3 ~ /^[a-zA-Z0-9_.-]+$/ { fs=$3 }
FILENAME=="/proc/net/dev" && $1 ~ /^[a-zA-Z0-9_.-]+:$/ && count<128 {
    name=substr($1,1,length($1)-1)
    net=net (count++ ? "," : "") sprintf("{\"name\":\"%s\",\"rx_bytes\":%s,\"tx_bytes\":%s,\"rx_bytes_per_second\":null,\"tx_bytes_per_second\":null}", name,$2,$10)
}
END {
    if (!cores || !mem["MemTotal:"] || !load) exit 1
    split(first,a); split(second,b); split(disk,d)
    cpu="null"
    if (b[1]>a[1]) {
        percent=100*(1-(b[2]-a[2])/(b[1]-a[1]))
        cpu=sprintf("%.2f",percent<0 ? 0 : (percent>100 ? 100 : percent))
    }
    printf "{\"outcome\":\"success\",\"value\":{\"os\":\"Linux\",\"arch\":\"%s\",\"sample\":{",arch
    printf "\"captured_at\":%.0f,\"logical_cores\":%d,\"cpu_percent\":%s,\"load\":%s,",stamp,cores,cpu,load
    printf "\"memory\":%s,\"swap\":%s,",capacity(mem["MemTotal:"],mem["MemAvailable:"]),capacity(mem["SwapTotal:"],mem["SwapFree:"])
    printf "\"filesystems\":[{\"mountpoint\":\"/\",\"fs_type\":\"%s\",\"total_bytes\":%.0f,\"available_bytes\":%.0f}],",fs ? fs : "rootfs",d[1],d[2]
    printf "\"interfaces\":[%s]}}}\n",net
}' /proc/stat /proc/meminfo /proc/loadavg /proc/mounts /proc/net/dev
