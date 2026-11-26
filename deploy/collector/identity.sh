deployment_identity() {
    boot=$(cat /proc/sys/kernel/random/boot_id) || return 1
    machine=$(cat /etc/machine-id 2>/dev/null) || machine=
    printf '%s\n' "$boot" "$machine" "$(id -un)" "$(id -u)" "$(uname -m)" | sha256sum
}
