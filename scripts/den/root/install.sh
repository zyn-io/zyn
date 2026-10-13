#!/usr/bin/env bash
# Epeaky: a den on this server, behind Caddy (TLS from Let's Encrypt).
#
#   curl -fsSL https://epeaky.zyn-network.com/install.sh | sudo bash
#   sudo bash install.sh --domain chat.example.org [--email you@example.org] [--yes]
#
# Needs: Linux (x86-64), a domain whose A/AAAA record points here, TCP 80 and
# 443 open, and UDP 50000-50199 for voice. Installs Docker if it is missing
# (after asking). Everything lives in /opt/epeaky; run it again to update.
set -euo pipefail

DIR=/opt/epeaky
IMAGE=${EPEAKY_IMAGE:-ghcr.io/zyn-io/epeaky:latest}
DOMAIN=${EPEAKY_DOMAIN:-}
EMAIL=${EPEAKY_EMAIL:-}
YES=0

while [ $# -gt 0 ]; do
  case "$1" in
    --domain) DOMAIN=${2:-}; shift 2 ;;
    --email) EMAIL=${2:-}; shift 2 ;;
    --yes|-y) YES=1; shift ;;
    -h|--help) sed -n '2,10p' "$0"; exit 0 ;;
    *) echo "unknown option: $1" >&2; exit 2 ;;
  esac
done

say() { printf '%s\n' "$*"; }
fail() { printf 'epeaky: %s\n' "$*" >&2; exit 1; }
# Questions read from the terminal, so `curl … | bash` can still ask.
ask() {
  local prompt=$1 var
  if [ "$YES" = 1 ] || [ ! -r /dev/tty ]; then return 1; fi
  printf '%s' "$prompt" > /dev/tty
  IFS= read -r var < /dev/tty || return 1
  printf '%s' "$var"
}

[ "$(id -u)" = 0 ] || fail "run it as root (sudo bash install.sh)"
[ "$(uname -s)" = Linux ] || fail "this installer is for Linux servers"
case "$(uname -m)" in x86_64|amd64) ;; *) fail "the image is built for x86-64 only so far; build it yourself with the repository's Dockerfile" ;; esac

# ---- Docker -------------------------------------------------------------------
if ! command -v docker >/dev/null 2>&1 || ! docker compose version >/dev/null 2>&1; then
  if [ "$YES" = 1 ]; then
    answer=y
  else
    answer=$(ask "Docker (with compose) is not installed. Install it now with Docker's own script from get.docker.com? [y/N] ") || answer=n
  fi
  case "$answer" in y|Y|yes) curl -fsSL https://get.docker.com | sh ;; *) fail "install Docker first: https://docs.docker.com/engine/install/" ;; esac
fi

# ---- The domain -----------------------------------------------------------------
if [ -f "$DIR/.env" ] && [ -z "$DOMAIN" ]; then
  DOMAIN=$(sed -n 's/^PHX_HOST=//p' "$DIR/.env")
fi
if [ -z "$DOMAIN" ]; then
  DOMAIN=$(ask "The den's domain (its A/AAAA record must point at this server), e.g. chat.example.org: ") || fail "give the domain with --domain"
fi
printf '%s' "$DOMAIN" | grep -Eq '^[A-Za-z0-9]([A-Za-z0-9-]*[A-Za-z0-9])?(\.[A-Za-z0-9]([A-Za-z0-9-]*[A-Za-z0-9])?)+$' || fail "that is not a domain name: $DOMAIN"

# Does the domain point here? A warning, not a stop: DNS may still be on its way.
here=$(curl -fsS -4 --max-time 5 https://api.ipify.org 2>/dev/null || true)
there=$(getent ahostsv4 "$DOMAIN" 2>/dev/null | awk 'NR==1 {print $1}' || true)
if [ -n "$here" ] && [ "$there" != "$here" ]; then
  say "Note: $DOMAIN resolves to '${there:-nothing}', and this server is $here. Caddy cannot get a certificate until it points here."
fi

# ---- /opt/epeaky ------------------------------------------------------------------
mkdir -p "$DIR/data" "$DIR/caddy"
# The den runs as uid 9100 in its image; its data is its own.
chown 9100 "$DIR/data" && chmod 700 "$DIR/data"

if [ ! -f "$DIR/.env" ]; then
  umask 077
  cat > "$DIR/.env" <<ENV
PHX_HOST=$DOMAIN
SECRET_KEY_BASE=$(head -c 48 /dev/urandom | base64 | tr -d '\n')
DATABASE_PATH=/var/lib/den/den.db
DEN_KEYS_DIR=/var/lib/den/keys
# The ways in: invite, approval, external (comma-separated).
DEN_ENTRY=invite
# The rooms every member sees, as id:Title,…
DEN_ROOMS=general:General
# Voice offers devices only this server's public addresses.
DEN_VOICE_PUBLIC_ONLY=1
ENV
  umask 022
else
  sed -i "s/^PHX_HOST=.*/PHX_HOST=$DOMAIN/" "$DIR/.env"
fi

cat > "$DIR/Caddyfile" <<CADDY
{
	${EMAIL:+email $EMAIL}
}

$DOMAIN {
	encode zstd gzip
	reverse_proxy 127.0.0.1:4000
}
CADDY

# Host networking: voice needs the server's real addresses, and the den
# listens on loopback only, behind Caddy.
cat > "$DIR/compose.yml" <<COMPOSE
services:
  den:
    image: $IMAGE
    network_mode: host
    env_file: .env
    volumes:
      - ./data:/var/lib/den
    restart: unless-stopped
  caddy:
    image: caddy:2
    network_mode: host
    volumes:
      - ./Caddyfile:/etc/caddy/Caddyfile:ro
      - ./caddy:/data
    restart: unless-stopped
COMPOSE

# ---- The firewall ---------------------------------------------------------------------
if command -v ufw >/dev/null 2>&1 && ufw status 2>/dev/null | grep -q '^Status: active'; then
  ufw allow 80/tcp >/dev/null && ufw allow 443/tcp >/dev/null && ufw allow 50000:50199/udp >/dev/null
  say "Firewall (ufw): opened 80/tcp, 443/tcp and 50000-50199/udp."
elif command -v firewall-cmd >/dev/null 2>&1 && firewall-cmd --state >/dev/null 2>&1; then
  firewall-cmd -q --permanent --add-service=http --add-service=https --add-port=50000-50199/udp && firewall-cmd -q --reload
  say "Firewall (firewalld): opened http, https and 50000-50199/udp."
else
  say "Open TCP 80 and 443, and UDP 50000-50199 for voice, in your server's firewall if it has one."
fi

# ---- The host's command ---------------------------------------------------------------
cat > /usr/local/bin/epeaky <<'CLI'
#!/usr/bin/env bash
# Epeaky's host commands, against the den in /opt/epeaky.
set -euo pipefail
cd /opt/epeaky
rpc() { docker compose exec -T den /app/bin/den rpc "$1"; }
q() { printf '"%s"' "${1//[^0-9a-f]/}"; }
case "${1:-help}" in
  invite)   rpc "Den.Host.invite(${2:-1}, ${3:-7})" ;;
  invites)  rpc "Den.Host.invites()" ;;
  withdraw) rpc "IO.inspect(Den.Host.withdraw($(q "${2:?an invite id}")))" ;;
  requests) rpc "Den.Host.requests()" ;;
  approve)  rpc "IO.inspect(Den.Host.approve($(q "${2:?a request id}")))" ;;
  decline)  rpc "IO.inspect(Den.Host.decline($(q "${2:?a request id}")))" ;;
  members)  rpc "IO.inspect(Den.Host.members())" ;;
  logs)     docker compose logs --tail "${2:-100}" den ;;
  update)   docker compose pull && docker compose up -d ;;
  *) cat <<HELP
epeaky invite [N] [DAYS]   N one-use invite links, good for DAYS days (1, 7)
epeaky invites             invites made, used, withdrawn
epeaky withdraw ID         withdraw an invite, and the pass it let in
epeaky requests            people asking to join (when approval is open)
epeaky approve ID          let a request in; epeaky decline ID turns it down
epeaky members             how many members, by the way they joined
epeaky logs [N]            the den's last N log lines
epeaky update              pull the latest image and restart
Settings: /opt/epeaky/.env (then: cd /opt/epeaky && docker compose up -d)
HELP
  ;;
esac
CLI
chmod 755 /usr/local/bin/epeaky

# ---- Start ------------------------------------------------------------------------------
cd "$DIR"
# EPEAKY_NO_PULL: an image already on this machine (tests, your own build).
[ -n "${EPEAKY_NO_PULL:-}" ] || docker compose pull -q
docker compose up -d
say "Starting the den…"
for _ in $(seq 1 60); do
  curl -fsS -o /dev/null -H 'X-Forwarded-Proto: https' http://127.0.0.1:4000/door/ways 2>/dev/null && break
  sleep 2
done
curl -fsS -o /dev/null -H 'X-Forwarded-Proto: https' http://127.0.0.1:4000/door/ways || fail "the den did not start; see: epeaky logs"

invite=$(docker compose logs den 2>/dev/null | grep -o 'https://[^ ]*#invite=[0-9a-f]\{64\}' | tail -1 || true)
say ""
say "Epeaky is running at https://$DOMAIN"
if [ -n "$invite" ]; then
  say "Your first invite (one use, 30 days; open it on the device you will use):"
  say "  $invite"
else
  say "Make an invite with: epeaky invite"
fi
say "More: epeaky help"
