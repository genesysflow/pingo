#!/usr/bin/env bash
# Self-contained Docker setup; the published site fills in the image address.
set -euo pipefail
umask 077

image=${PINGO_IMAGE:-ghcr.io/genesysflow/pingo:latest}
install_dir=./pingo
domain=
public_ip=
http_port=8090
udp_port=40000
https=1
start=1
turn_urls=

die() { printf 'Error: %s\n' "$*" >&2; exit 1; }
usage() {
  cat <<'HELP'
Pingo Docker setup

Usage: bash install.sh [options]
  --domain DOMAIN      Your Pingo domain (prompted if omitted)
  --public-ip IP       Reachable server IP; comma-separated for multiple IPs
  --image IMAGE        Published container image (or set PINGO_IMAGE)
  --dir DIRECTORY      Install directory (default: ./pingo)
  --http-port PORT     HTTP port behind the proxy (default: 8090)
  --udp-port PORT      Voice/video UDP port (default: 40000)
  --turn-urls URLS     Enable an external TURN relay; comma-separated turn:/turns: URLs
  --no-https           Use your existing HTTPS reverse proxy instead of Caddy
  --no-start           Write and validate configuration without starting Docker services
  --help               Show this help

Requires Docker with the docker compose command available. Existing configuration is never overwritten.
HELP
}
while (($#)); do
  case "$1" in
    --domain|--public-ip|--image|--dir|--http-port|--udp-port|--turn-urls)
      (($# >= 2)) || die "Missing value for $1"
      case "$1" in
        --turn-urls) turn_urls=$2 ;;
        --domain) domain=$2 ;;
        --public-ip) public_ip=$2 ;;
        --image) image=$2 ;;
        --dir) install_dir=$2 ;;
        --http-port) http_port=$2 ;;
        --udp-port) udp_port=$2 ;;
      esac
      shift 2 ;;
    --no-https) https=0; shift ;;
    --no-start) start=0; shift ;;
    --help|-h) usage; exit 0 ;;
    *) die "Unknown option: $1 (see --help)" ;;
  esac
done

command -v docker >/dev/null || die 'Install Docker first: https://docs.docker.com/engine/install/'
docker compose version >/dev/null 2>&1 || die 'The docker compose command is unavailable. See: https://docs.docker.com/compose/install/'
if ((start)); then
  docker info >/dev/null 2>&1 || die 'Docker is not available. Start it and make sure your user can run docker.'
fi

# Read the terminal directly so prompts also work when the script is piped to bash.
prompt() {
  local answer
  if ! { printf '%s: ' "$1" > /dev/tty; IFS= read -r answer < /dev/tty; } 2>/dev/null; then
    die "Cannot prompt here; provide $2 on the command line."
  fi
  printf '%s' "$answer"
}
[[ -n $domain ]] || domain=$(prompt 'Pingo domain (e.g. chat.example.com)' --domain)
[[ -n $public_ip ]] || public_ip=$(prompt 'Server public IP (e.g. 203.0.113.10)' --public-ip)

domain_pattern='^([a-zA-Z0-9]([a-zA-Z0-9-]{0,61}[a-zA-Z0-9])?\.)+[a-zA-Z0-9]([a-zA-Z0-9-]{0,61}[a-zA-Z0-9])?$'
[[ $domain =~ $domain_pattern ]] || die 'Use a domain name, e.g. chat.example.com, without https:// or a path.'
[[ -n $public_ip && $public_ip != *[!0-9a-fA-F:.,]* && $public_ip != ,* && $public_ip != *, && $public_ip != *,,* ]] || die 'Use IP addresses separated by commas, without spaces.'
[[ -n $image && $image != *[!a-zA-Z0-9._/:@-]* ]] || die 'Invalid container image address.'
for port in "$http_port" "$udp_port"; do
  [[ $port =~ ^[0-9]{1,5}$ ]] && ((10#$port > 0 && 10#$port <= 65535)) || die 'Ports must be between 1 and 65535.'
done
# URLs go into a dotenv file; reject shell/Compose interpolation and line injection.
[[ $turn_urls != *[!a-zA-Z0-9.:?=,\[\]-]* ]] || die 'Invalid TURN URLs.'
[[ -n $install_dir ]] || die 'The install directory cannot be empty.'
mkdir -p -- "$install_dir"
cd -- "$install_dir"
for file in .env compose.yaml Caddyfile turn-entrypoint.sh; do
  [[ ! -e $file && ! -L $file ]] || die "$PWD/$file already exists. Use its docker compose commands or choose another --dir."
done

if command -v openssl >/dev/null; then
  media_secret=$(openssl rand -hex 32)
else
  media_secret=$(od -An -N32 -tx1 /dev/urandom | tr -d ' \n')
fi
[[ ${#media_secret} == 64 ]] || die 'Could not generate a media secret.'
turn_secret=
if [[ -n $turn_urls ]]; then
  if command -v openssl >/dev/null; then turn_secret=$(openssl rand -hex 32)
  else turn_secret=$(od -An -N32 -tx1 /dev/urandom | tr -d ' \n'); fi
  [[ ${#turn_secret} == 64 ]] || die 'Could not generate a TURN secret.'
fi
cat > .env <<ENV
PINGO_IMAGE=$image
PINGO_URL=https://$domain
PINGO_DOMAIN=$domain
PINGO_PUBLIC_IPS=$public_ip
PINGO_MEDIA_SECRET=$media_secret
PINGO_TURN_URLS=$turn_urls
PINGO_TURN_SECRET=$turn_secret
PINGO_HTTP_PORT=$http_port
PINGO_UDP_PORT=$udp_port
PINGO_REGISTRATION=invite
RXFERRIX_TRUST_PROXY=true
ENV

cat > compose.yaml <<'COMPOSE'
# Standalone deployment: no source checkout or local build required.
# Configure .env, then: docker compose --profile https up -d
services:
  pingo:
    image: ${PINGO_IMAGE:?Set PINGO_IMAGE in .env to the published Pingo image}
    restart: unless-stopped
    env_file: .env
    environment:
      RXFERRIX_PUBLIC_URL: ${PINGO_URL:?Set PINGO_URL in .env}
      PINGO_UDP_PORT: ${PINGO_UDP_PORT:-40000}
    ports:
      - "${PINGO_HTTP_PORT:-8090}:8090/tcp"
      - "${PINGO_UDP_PORT:-40000}:${PINGO_UDP_PORT:-40000}/udp"
    volumes:
      - pingo-data:/data

  turn:
    profiles: [turn]
    image: coturn/coturn:4.6.3
    restart: unless-stopped
    # Linux host networking avoids a Docker rule for every relay UDP port.
    network_mode: host
    env_file: .env
    entrypoint: ["/bin/sh", "/etc/coturn/entrypoint.sh"]
    volumes:
      - ./turn-entrypoint.sh:/etc/coturn/entrypoint.sh:ro
      - ${PINGO_TURN_CERTS_DIR:-./turn-certs}:/etc/coturn/certs:ro

  caddy:
    profiles: [https]
    image: caddy:2
    restart: unless-stopped
    environment:
      PINGO_DOMAIN: ${PINGO_DOMAIN:-}
    ports:
      - "80:80"
      - "443:443"
      - "443:443/udp"
    volumes:
      - ./Caddyfile:/etc/caddy/Caddyfile:ro
      - caddy-data:/data
      - caddy-config:/config
    depends_on: [pingo]

volumes:
  pingo-data:
  caddy-data:
  caddy-config:
COMPOSE
cat > turn-entrypoint.sh <<'TURN_ENTRYPOINT'
#!/bin/sh
# Render coturn configuration inside the container; keep the shared secret out of argv.
set -eu
umask 077
: "${PINGO_TURN_SECRET:?Set PINGO_TURN_SECRET}"
: "${PINGO_TURN_RELAY_IP:?Set PINGO_TURN_RELAY_IP to an address assigned to this host}"
: "${PINGO_TURN_PEER_IPS:?Set PINGO_TURN_PEER_IPS to the SFU candidate IPs}"
for value in "$PINGO_TURN_SECRET" "$PINGO_TURN_RELAY_IP" "$PINGO_TURN_PEER_IPS" "${PINGO_TURN_PUBLIC_IP:-}" "${PINGO_TURN_REALM:-pingo}"; do
  case "$value" in *[!a-zA-Z0-9._,:/-]*) echo 'Invalid TURN setting' >&2; exit 2 ;; esac
done
for port in "${PINGO_TURN_PORT:-3478}" "${PINGO_TURN_MIN_PORT:-49160}" "${PINGO_TURN_MAX_PORT:-49200}" "${PINGO_TURN_TLS_PORT:-5349}"; do
  case "$port" in ''|*[!0-9]*) echo 'Invalid TURN port' >&2; exit 2 ;; esac
  [ "$port" -gt 0 ] && [ "$port" -le 65535 ] || exit 2
done
[ "${PINGO_TURN_MIN_PORT:-49160}" -le "${PINGO_TURN_MAX_PORT:-49200}" ] || exit 2
conf=$(mktemp)
trap 'rm -f "$conf"' EXIT
cat > "$conf" <<EOF
listening-ip=$PINGO_TURN_RELAY_IP
relay-ip=$PINGO_TURN_RELAY_IP
listening-port=${PINGO_TURN_PORT:-3478}
min-port=${PINGO_TURN_MIN_PORT:-49160}
max-port=${PINGO_TURN_MAX_PORT:-49200}
realm=${PINGO_TURN_REALM:-pingo}
use-auth-secret
static-auth-secret=$PINGO_TURN_SECRET
fingerprint
no-cli
no-multicast-peers
no-tcp-relay
no-dtls
no-tlsv1
no-tlsv1_1
user-quota=8
total-quota=200
max-bps=2000000
bps-capacity=100000000
denied-peer-ip=0.0.0.0-255.255.255.255
denied-peer-ip=::-ffff:ffff:ffff:ffff:ffff:ffff:ffff:ffff
log-file=stdout
simple-log
EOF
if [ -n "${PINGO_TURN_PUBLIC_IP:-}" ]; then
  printf 'external-ip=%s/%s\n' "$PINGO_TURN_PUBLIC_IP" "$PINGO_TURN_RELAY_IP" >> "$conf"
fi
previous_ifs=$IFS
IFS=,
for peer in $PINGO_TURN_PEER_IPS; do
  [ -n "$peer" ] || exit 2
  printf 'allowed-peer-ip=%s\n' "$peer" >> "$conf"
done
IFS=$previous_ifs
if [ -n "${PINGO_TURN_TLS_PORT:-}" ]; then
  if ! test -r /etc/coturn/certs/fullchain.pem || ! test -r /etc/coturn/certs/privkey.pem; then
    echo 'TURN TLS needs readable /etc/coturn/certs/fullchain.pem and privkey.pem' >&2
    exit 2
  fi
  cat >> "$conf" <<EOF
tls-listening-port=$PINGO_TURN_TLS_PORT
cert=/etc/coturn/certs/fullchain.pem
pkey=/etc/coturn/certs/privkey.pem
EOF
else
  printf 'no-tls\n' >> "$conf"
fi
# Preserve a PID 1 turnserver for graceful shutdown; the temporary file lives only in this container.
exec turnserver -c "$conf"
TURN_ENTRYPOINT
cat > Caddyfile <<'CADDY'
# Automatic HTTPS for Pingo. Browsers only allow microphones and screen sharing over HTTPS.
{$PINGO_DOMAIN} {
	encode zstd gzip
	reverse_proxy pingo:8090
}
CADDY

compose=(docker compose)
if ((https)); then compose+=(--profile https); fi
# Explicit options must win over any Compose interpolation variables in the caller's shell.
export PINGO_IMAGE="$image" PINGO_URL="https://$domain" PINGO_DOMAIN="$domain"
export PINGO_HTTP_PORT="$http_port" PINGO_UDP_PORT="$udp_port"
"${compose[@]}" config --quiet
printf '\nConfiguration written to %s\n' "$PWD"
printf 'Registration is invite-only; the first account can create the first space.\n'
if [[ -n $turn_urls ]]; then
  printf 'TURN credentials are enabled. Configure your coturn with PINGO_TURN_SECRET from .env before testing calls. See docs/SELF_HOSTING.md.\n'
fi
if ((!start)); then
  printf 'To start: cd %q && ' "$PWD"
  printf '%q ' "${compose[@]}"
  printf 'pull && '
  printf '%q ' "${compose[@]}"
  printf 'up -d\n'
  exit 0
fi
"${compose[@]}" pull
"${compose[@]}" up -d --wait --wait-timeout 120
printf '\nPingo is running. Open https://%s to create the first account.\n' "$domain"
if ((https)); then
  printf 'Point your domain at this server and allow 80/tcp, 443/tcp and %s/udp.\n' "$udp_port"
else
  printf 'Configure your HTTPS reverse proxy to forward to port %s, including WebSockets. Allow %s/udp for calls.\n' "$http_port" "$udp_port"
fi
