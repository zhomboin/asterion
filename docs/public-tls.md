# Let's Encrypt 公网证书与自动续期

[一键初始化中心](deployment.md#一键初始化中心)会签发或复用证书、启用定时器并验证模拟续期。本页供单独签发、迁移及日常维护时使用。

在中心服务器的 root Bash 中执行。使用第 2 节设置的 `domain`，或将它设为你的面板域名。

## 准备 HTTP 验证

域名的 A 记录应指向中心公网 IPv4。如果发布了 AAAA 记录，该 IPv6 地址也应能提供验证路径。确保公网 TCP 80 的 `/.well-known/acme-challenge/` 请求能到达本机 Nginx，包括经过安全组、防火墙或 CDN 的情况。TCP 443 提供面板的 HTTPS 连接。自动续期同样需要验证路径可达。[Let’s Encrypt 的 HTTP-01 验证](https://letsencrypt.org/zh-cn/docs/challenge-types/#http-01-挑战)

```bash
set -euo pipefail
test "$(id -u)" = 0
: "${domain:?请先设置面板域名 domain}"
apt-get update
apt-get install -y nginx certbot python3-certbot-nginx
nginx -t
systemctl enable --now nginx
```

## 签发证书并启用自动续期

新签证书时填写自己能收信的常用邮箱，Certbot 会自动注册或复用账户。已有同名 Certbot 证书沿用现有验证方式，包括 DNS 验证所需的插件和凭据。[Certbot 使用说明](https://eff-certbot.readthedocs.io/en/stable/using.html)

```bash
if [[ -f /etc/letsencrypt/renewal/$domain.conf ]]; then
  /usr/bin/certbot certificates --cert-name "$domain"
else
  read -rp 'Let’s Encrypt 联系邮箱：' acme_email </dev/tty
  /usr/bin/certbot certonly --nginx --preferred-challenges http-01 \
    --non-interactive --agree-tos --email "$acme_email" --cert-name "$domain" -d "$domain" \
    --deploy-hook '/usr/sbin/nginx -t && /usr/bin/systemctl reload nginx'
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

首次部署请先回到[服务器部署](deployment.md#一键初始化中心)，启用域名站点后再进行本项验收。迁移主机时还需按[备份和恢复](deployment.md#备份和恢复)在新主机验证自动续期。

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
