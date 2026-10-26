#!/usr/bin/env bash
# Self-contained Docker setup; the published site fills in the image address.
set -euo pipefail
umask 077

image=${PINGO_IMAGE:-ghcr.io/OWNER/REPOSITORY:latest}
install_dir=./pingo
domain=
public_ip=
http_port=8090
udp_port=40000
https=1
start=1

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
  --no-https           Use your existing HTTPS reverse proxy instead of Caddy
  --no-start           Write and validate configuration without starting Docker services
  --help               Show this help

Requires Docker with the Compose plugin. Existing configuration is never overwritten.
HELP
}
while (($#)); do
  case "$1" in
    --domain|--public-ip|--image|--dir|--http-port|--udp-port)
      (($# >= 2)) || die "Missing value for $1"
      case "$1" in
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

command -v docker >/dev/null || die 'Install Docker Engine with its Compose plugin first: https://docs.docker.com/engine/install/'
docker compose version >/dev/null 2>&1 || die 'Install the Docker Compose plugin first: https://docs.docker.com/compose/install/'
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
if [[ $image == *OWNER/REPOSITORY* ]]; then
  image=$(prompt 'Published Pingo container image' --image)
fi

domain_pattern='^([a-zA-Z0-9]([a-zA-Z0-9-]{0,61}[a-zA-Z0-9])?\.)+[a-zA-Z0-9]([a-zA-Z0-9-]{0,61}[a-zA-Z0-9])?$'
[[ $domain =~ $domain_pattern ]] || die 'Use a domain name, e.g. chat.example.com, without https:// or a path.'
[[ -n $public_ip && $public_ip != *[!0-9a-fA-F:.,]* && $public_ip != ,* && $public_ip != *, && $public_ip != *,,* ]] || die 'Use IP addresses separated by commas, without spaces.'
[[ -n $image && $image != *[!a-zA-Z0-9._/:@-]* ]] || die 'Invalid container image address.'
for port in "$http_port" "$udp_port"; do
  [[ $port =~ ^[0-9]{1,5}$ ]] && ((10#$port > 0 && 10#$port <= 65535)) || die 'Ports must be between 1 and 65535.'
done
[[ -n $install_dir ]] || die 'The install directory cannot be empty.'
mkdir -p -- "$install_dir"
cd -- "$install_dir"
for file in .env compose.yaml Caddyfile; do
  [[ ! -e $file && ! -L $file ]] || die "$PWD/$file already exists. Use its docker compose commands or choose another --dir."
done

if command -v openssl >/dev/null; then
  media_secret=$(openssl rand -hex 32)
else
  media_secret=$(od -An -N32 -tx1 /dev/urandom | tr -d ' \n')
fi
[[ ${#media_secret} == 64 ]] || die 'Could not generate a media secret.'
cat > .env <<ENV
PINGO_IMAGE=$image
PINGO_URL=https://$domain
PINGO_DOMAIN=$domain
PINGO_PUBLIC_IPS=$public_ip
PINGO_MEDIA_SECRET=$media_secret
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
