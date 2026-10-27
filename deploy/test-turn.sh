#!/usr/bin/env bash
# Local integration only: real coturn, no public server or audio devices.
set -euo pipefail
cd "$(dirname "$0")/.."
export PATH="$HOME/.cargo/bin:$PATH"
for cmd in docker openssl python3 cargo; do command -v "$cmd" >/dev/null; done
turn_tmp=$(mktemp -d)
turn_container="pingo-turn-test-$$"
# Match the reachable SFU address and exercise the actual production entrypoint and peer ACL.
export PINGO_TEST_MEDIA_IP
PINGO_TEST_MEDIA_IP=$(python3 - <<'PYIP'
import socket
with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as probe:
 probe.connect(('192.0.2.1', 9))
 print(probe.getsockname()[0])
PYIP
)
cleanup() { docker rm -f "$turn_container" >/dev/null 2>&1 || true; rm -rf "$turn_tmp"; }
trap cleanup EXIT
# Pick unused listener ports; relay allocations use ephemeral ports.
read -r udp_port tls_port < <(python3 - <<'PY'
import socket
with socket.socket() as a, socket.socket() as b:
 a.bind(('127.0.0.1',0)); b.bind(('127.0.0.1',0))
 print(a.getsockname()[1], b.getsockname()[1])
PY
)
openssl req -x509 -newkey rsa:2048 -nodes -days 1 -subj /CN=Pingo-Test-CA \
  -keyout "$turn_tmp/ca.key" -out "$turn_tmp/ca.pem" >/dev/null 2>&1
openssl req -newkey rsa:2048 -nodes -subj /CN=localhost \
  -keyout "$turn_tmp/privkey.pem" -out "$turn_tmp/request.pem" >/dev/null 2>&1
cat > "$turn_tmp/extensions" <<'EXT'
basicConstraints=critical,CA:FALSE
keyUsage=critical,digitalSignature,keyEncipherment
extendedKeyUsage=serverAuth
subjectAltName=DNS:localhost,IP:127.0.0.1,IP:TURN_TEST_IP
EXT
sed -i "s/TURN_TEST_IP/$PINGO_TEST_MEDIA_IP/" "$turn_tmp/extensions"
openssl x509 -req -in "$turn_tmp/request.pem" -CA "$turn_tmp/ca.pem" -CAkey "$turn_tmp/ca.key" \
  -CAcreateserial -days 1 -extfile "$turn_tmp/extensions" -out "$turn_tmp/fullchain.pem" >/dev/null 2>&1
chmod 755 "$turn_tmp"
chmod 644 "$turn_tmp/privkey.pem" "$turn_tmp/fullchain.pem"
export PINGO_TEST_TURN_SECRET
PINGO_TEST_TURN_SECRET=$(openssl rand -hex 32)
docker run -d --name "$turn_container" --network host \
  -v "$PWD/deploy/turn-entrypoint.sh:/etc/coturn/entrypoint.sh:ro" \
  -v "$turn_tmp:/etc/coturn/certs:ro" \
  -e PINGO_TURN_SECRET="$PINGO_TEST_TURN_SECRET" \
  -e PINGO_TURN_RELAY_IP="$PINGO_TEST_MEDIA_IP" -e PINGO_TURN_PEER_IPS="$PINGO_TEST_MEDIA_IP" \
  -e PINGO_TURN_PORT="$udp_port" -e PINGO_TURN_TLS_PORT="$tls_port" \
  --entrypoint /bin/sh coturn/coturn:4.6.3 /etc/coturn/entrypoint.sh > /dev/null
# Wait on the listener rather than sleeping for a guessed startup interval.
python3 - "$PINGO_TEST_MEDIA_IP" "$udp_port" <<'PY'
import socket,sys,time
for _ in range(100):
 try:
  with socket.create_connection((sys.argv[1],int(sys.argv[2])), timeout=.1): break
 except OSError: time.sleep(.1)
else: raise SystemExit('coturn did not start')
PY
export SSL_CERT_FILE="$turn_tmp/ca.pem"
for url in "turn:$PINGO_TEST_MEDIA_IP:$udp_port?transport=udp" "turn:$PINGO_TEST_MEDIA_IP:$udp_port?transport=tcp" "turns:$PINGO_TEST_MEDIA_IP:$tls_port?transport=tcp"; do
  export PINGO_TEST_TURN_URL="$url"
  cargo test -p pingo-media --test turn coturn_native_audio_and_data -- --ignored --nocapture
  cargo test -p pingo-media --test turn coturn_rejects_bad_credentials -- --ignored --nocapture
  PINGO_TEST_TURN_EXPIRED=1 cargo test -p pingo-media --test turn coturn_rejects_bad_credentials -- --ignored --nocapture
  cargo test -p pingo-media --lib coturn_restricts_peer_destinations -- --ignored --nocapture
  # Text/Motion screen video against the actual RxFerrix backend, over each relay transport.
  cargo test -p pingo-media --test native_video native_video_full_server_text_and_motion -- --nocapture
done
# Removing the test CA must make the native TLS handshake fail.
SSL_CERT_FILE=/dev/null PINGO_TEST_TURN_UNTRUSTED=1 \
  cargo test -p pingo-media --test turn coturn_rejects_bad_credentials -- --ignored --nocapture
if [[ ${1:-} == --browser ]]; then
  export PINGO_TURN_URLS="turn:$PINGO_TEST_MEDIA_IP:$udp_port?transport=tcp"
  export PINGO_TURN_SECRET="$PINGO_TEST_TURN_SECRET" PINGO_E2E_RELAY_ONLY=1 PINGO_E2E_MEDIA_IP="$PINGO_TEST_MEDIA_IP"
  cargo build -p pingo-server
  (cd apps/web && bun run test:e2e tests/e2e/real/calls.spec.ts)
fi
