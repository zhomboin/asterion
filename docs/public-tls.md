# Let's Encrypt 公网证书与自动续期

[一键初始化中心](deployment.md#一键初始化中心)会签发或复用证书、启用定时器并验证模拟续期。本页供单独签发、迁移及日常维护时使用。

在中心服务器的 root Bash 中执行。首次安装时沿用[服务器部署第 2 节](deployment.md#2-配置服务器与个人运维用户)设置的 `domain`。已部署的中心可单独执行本页，先把 `domain` 设为实际面板域名。以下命令适用于 Ubuntu 22.04 或 24.04、已运行的 Nginx 和由 Cloudflare 托管的权威 DNS。其他 DNS 服务商需选用对应的 Certbot DNS 插件和凭据格式。DNS-01 通过公网 DNS 的 TXT 记录完成验证，无需开放新的入站端口。[Let's Encrypt 验证方式](https://letsencrypt.org/docs/challenge-types/) · [Certbot DNS 插件](https://eff-certbot.readthedocs.io/en/stable/using.html#dns-plugins)

## 准备 DNS 授权

在 Cloudflare 创建专用 API Token，权限选择 `Zone → DNS → Edit`，资源限制为面板域名所在的单个区域。把令牌输入中心服务器的隐藏提示。Certbot 首次签发和后续续期均从这个文件读取令牌。[Cloudflare 插件说明](https://certbot-dns-cloudflare.readthedocs.io/en/stable/)

```bash
set -euo pipefail
test "$(id -u)" = 0
: "${domain:?请先设置面板域名 domain}"
apt-get update
apt-get install -y certbot python3-certbot-dns-cloudflare
install -d -m 0700 /etc/letsencrypt/asterion
credentials=/etc/letsencrypt/asterion/cloudflare.ini
if [[ ! -s $credentials ]]; then
  (
    set +x
    umask 077
    IFS= read -rsp 'Cloudflare API Token：' api_token </dev/tty
    printf '\n'
    test -n "$api_token"
    printf 'dns_cloudflare_api_token = %s\n' "$api_token" > "$credentials"
  )
fi
chown root:root "$credentials"
chmod 0600 "$credentials"
```

## 签发证书并启用自动续期

使用与面板域名相同的证书名称。已有证书会沿用其续期配置。需要更换验证方式时，请先确认本机 Certbot 支持对应的配置命令。已有多个证书时，可先运行 `/usr/bin/certbot certificates` 核对名称与覆盖的域名。以下流程面向面板专用证书；共用证书的所有域名均需相应 DNS 授权。

```bash
dns_options=(
  --cert-name "$domain"
  --dns-cloudflare --dns-cloudflare-credentials "$credentials"
  --dns-cloudflare-propagation-seconds 60
  --preferred-challenges dns
  --deploy-hook '/usr/sbin/nginx -t && /usr/bin/systemctl reload nginx'
  --non-interactive
)
if [[ -f /etc/letsencrypt/renewal/$domain.conf ]]; then
  /usr/bin/certbot certificates --cert-name "$domain"
else
  read -rp 'Let’s Encrypt 注册邮箱：' acme_email </dev/tty
  /usr/bin/certbot certonly "${dns_options[@]}" --agree-tos --email "$acme_email" -d "$domain" \
    --server https://acme-v02.api.letsencrypt.org/directory
fi
systemctl enable --now certbot.timer
systemctl is-enabled certbot.timer
systemctl is-active certbot.timer
systemctl list-timers --all certbot.timer
/usr/bin/certbot renew --cert-name "$domain" --dry-run
/usr/sbin/nginx -t
```

确认定时器为 `enabled`、`active` 且有下次运行时间，模拟续期成功并通过 Nginx 配置检查。续期成功后，钩子先校验 Nginx 配置，再平滑重载。模拟续期使用测试环境，最后一条命令单独检查 Nginx 配置。定时任务记录可用 `journalctl -u certbot.service -n 50 --no-pager` 查看。[Certbot 续期说明](https://eff-certbot.readthedocs.io/en/stable/using.html#renewing-certificates)

## 验证 Nginx 实际提供的证书

首次部署请先回到[服务器部署](deployment.md#3-安装中心配置-nginx-并开启自监控)，启用域名站点后再进行本项验收。迁移主机时还需按[备份和恢复](deployment.md#6-备份和恢复)在新主机验证自动续期。

Nginx 的 `ssl_certificate` 使用 `/etc/letsencrypt/live/<面板域名>/fullchain.pem`，`ssl_certificate_key` 使用同目录的 `privkey.pem`。保存站点配置后执行以下命令，从回环地址带上域名校验源站证书及实际指纹。

```bash
nginx -t
systemctl reload nginx
served=$(openssl s_client -connect 127.0.0.1:443 -servername "$domain" \
  -verify_hostname "$domain" -verify_return_error -CAfile /etc/ssl/certs/ca-certificates.crt </dev/null)
served_fp=$(openssl x509 -noout -fingerprint -sha256 <<< "$served")
disk_fp=$(openssl x509 -noout -fingerprint -sha256 -in "/etc/letsencrypt/live/$domain/cert.pem")
test "$served_fp" = "$disk_fp"
openssl x509 -noout -subject -issuer -dates <<< "$served"
```
