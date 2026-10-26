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
  test -r /etc/coturn/certs/fullchain.pem && test -r /etc/coturn/certs/privkey.pem
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
