#!/bin/sh
# Workroam: deploy self-hosted storage behind Caddy with one command (PRD DEP-01).
#
#   curl -fsSL https://raw.githubusercontent.com/kverona-ai/workroam/main/deploy/install.sh \
#     | sh -s -- --domain s3.example.com [--storage rustfs|garage|seaweedfs] [--with-server]
#
# It writes a Docker Compose project (default /opt/workroam, or ~/workroam
# without root), starts the storage and Caddy (automatic HTTPS on 443), and
# prints what to enter in the Workroam setup wizard. Run it again with the
# same --dir to update the images; existing keys are kept.
set -eu

REPO="kverona-ai/workroam"
REF="${WORKROAM_REF:-main}"
DOMAIN=""
STORAGE="rustfs"
WITH_SERVER=0
WITH_GITEA=0
TUNNEL=""
BUCKET="workroam"
NTFY=""
START=1
if [ "$(id -u)" = 0 ]; then DIR="/opt/workroam"; else DIR="$HOME/workroam"; fi

usage() {
  cat <<EOF
Usage: install.sh --domain <domain> [options]

  --domain <domain>      public domain that points to this server (required)
  --storage <name>       rustfs (default), garage or seaweedfs
  --with-server          also run workroam-server (pairing codes, per-device
                         keys, phone notifications, web reader at /web/)
  --with-gitea           also run Gitea at /git/ (transfer repos for git
                         repos in \`remote\` mode)
  --tunnel <token>       no public IP: carry traffic over a Cloudflare Tunnel
  --bucket <name>        bucket to create for Garage / SeaweedFS (default workroam)
  --ntfy <url>           ntfy topic for phone notifications (with --with-server)
  --dir <path>           where to keep the deployment (default $DIR)
  --ref <git ref>        version of the deploy files to use (default $REF)
  --no-start             write the files but do not start anything
EOF
}

die() { printf 'install.sh: %s\n' "$*" >&2; exit 1; }
say() { printf '\033[1m==>\033[0m %s\n' "$*"; }

while [ $# -gt 0 ]; do
  case "$1" in
    --domain) DOMAIN="${2:-}"; shift 2 ;;
    --domain=*) DOMAIN="${1#*=}"; shift ;;
    --storage) STORAGE="${2:-}"; shift 2 ;;
    --storage=*) STORAGE="${1#*=}"; shift ;;
    --with-server) WITH_SERVER=1; shift ;;
    --with-gitea) WITH_GITEA=1; shift ;;
    --tunnel) TUNNEL="${2:-}"; shift 2 ;;
    --tunnel=*) TUNNEL="${1#*=}"; shift ;;
    --bucket) BUCKET="${2:-}"; shift 2 ;;
    --bucket=*) BUCKET="${1#*=}"; shift ;;
    --ntfy) NTFY="${2:-}"; shift 2 ;;
    --ntfy=*) NTFY="${1#*=}"; shift ;;
    --dir) DIR="${2:-}"; shift 2 ;;
    --dir=*) DIR="${1#*=}"; shift ;;
    --ref) REF="${2:-}"; shift 2 ;;
    --ref=*) REF="${1#*=}"; shift ;;
    --no-start) START=0; shift ;;
    -h|--help) usage; exit 0 ;;
    *) usage >&2; die "unknown option: $1" ;;
  esac
done

[ -n "$DOMAIN" ] || { usage >&2; die "--domain is required"; }
case "$DOMAIN" in
  *://*|*/*) die "--domain takes a host name such as s3.example.com, not a URL" ;;
esac
case "$STORAGE" in
  rustfs) UPSTREAM="storage:9000" ;;
  garage) UPSTREAM="storage:3900" ;;
  seaweedfs) UPSTREAM="storage:8333" ;;
  *) die "--storage must be rustfs, garage or seaweedfs" ;;
esac
printf '%s' "$BUCKET" | grep -Eq '^[a-z0-9][a-z0-9-]{1,61}[a-z0-9]$' \
  || die "--bucket: 3-63 lowercase letters, digits and hyphens"

if [ "$START" = 1 ]; then
  command -v docker >/dev/null 2>&1 || die "Docker is required: https://docs.docker.com/engine/install/"
  docker compose version >/dev/null 2>&1 || die "Docker Compose v2 is required (the \`docker compose\` plugin)"
fi

rand() { # rand <bytes> -> hex
  if command -v openssl >/dev/null 2>&1; then openssl rand -hex "$1"
  else od -An -tx1 -N"$1" /dev/urandom | tr -d ' \n'; fi
}
key_id() { printf 'WR%s' "$(rand 9 | tr 'a-f' 'A-F')"; }

# ---- deploy files: from this checkout, or downloaded for --ref ----------

SRC=""
case "$0" in
  */install.sh) [ -f "$(dirname "$0")/caddy/Caddyfile" ] && SRC="$(cd "$(dirname "$0")" && pwd)" ;;
esac
fetch() { # fetch <path under deploy/> <dest>
  if [ -n "$SRC" ]; then cp "$SRC/$1" "$2"; return; fi
  url="https://raw.githubusercontent.com/$REPO/$REF/deploy/$1"
  if command -v curl >/dev/null 2>&1; then curl -fsSL "$url" -o "$2"
  elif command -v wget >/dev/null 2>&1; then wget -qO "$2" "$url"
  else die "curl or wget is required"; fi
}

umask 077
mkdir -p "$DIR"
cd "$DIR"
say "Writing the deployment to $DIR"

# Keep the keys of an earlier run.
if [ -f .env ]; then
  # shellcheck disable=SC1091
  . ./.env
  [ "${WORKROAM_STORAGE:-$STORAGE}" = "$STORAGE" ] \
    || die "$DIR already holds a ${WORKROAM_STORAGE} deployment; use another --dir"
fi
STORAGE_ADMIN_ACCESS_KEY="${STORAGE_ADMIN_ACCESS_KEY:-$(key_id)}"
STORAGE_ADMIN_SECRET_KEY="${STORAGE_ADMIN_SECRET_KEY:-$(rand 20)}"

files="caddy.yml"
[ -n "$TUNNEL" ] || files="$files caddy-ports.yml"
files="$files $STORAGE.yml"
[ "$WITH_SERVER" = 0 ] || files="$files server.yml"
[ "$WITH_GITEA" = 0 ] || files="$files gitea.yml"
[ -z "$TUNNEL" ] || files="$files cloudflared.yml"
compose_file=""
for f in $files; do
  fetch "compose/$f" "$f"
  compose_file="${compose_file:+$compose_file:}$f"
done

# Caddyfile: keep the enabled optional blocks.
fetch caddy/Caddyfile Caddyfile.in
drop=""
[ "$WITH_SERVER" = 1 ] || drop="$drop server"
[ "$WITH_GITEA" = 1 ] || drop="$drop gitea"
awk -v drop="$drop" '
  BEGIN { n = split(drop, d, " "); for (i = 1; i <= n; i++) off[d[i]] = 1 }
  /^[ \t]*# >/ { s = $0; sub(/^[ \t]*# >/, "", s); if (s in off) skip = 1; next }
  /^[ \t]*# </ { skip = 0; next }
  !skip { print }
' Caddyfile.in > Caddyfile
rm -f Caddyfile.in
chmod 644 Caddyfile

SERVER_ADMIN_ACCESS_KEY=""
SERVER_ADMIN_SECRET_KEY=""
SERVER_ADMIN_ENDPOINT=""
SERVER_ADMIN_TOKEN=""
case "$STORAGE" in
  rustfs)
    SERVER_ADMIN_ACCESS_KEY="$STORAGE_ADMIN_ACCESS_KEY"
    SERVER_ADMIN_SECRET_KEY="$STORAGE_ADMIN_SECRET_KEY"
    SERVER_ADMIN_ENDPOINT="http://storage:9000"
    ;;
  garage)
    GARAGE_RPC_SECRET="${GARAGE_RPC_SECRET:-$(rand 32)}"
    GARAGE_ADMIN_TOKEN="${GARAGE_ADMIN_TOKEN:-$(rand 24)}"
    fetch compose/garage.toml garage.toml.in
    sed -e "s/@RPC_SECRET@/$GARAGE_RPC_SECRET/" \
        -e "s/@ADMIN_TOKEN@/$GARAGE_ADMIN_TOKEN/" \
        -e "s/@METRICS_TOKEN@/$(rand 16)/" garage.toml.in > garage.toml
    rm -f garage.toml.in
    chmod 644 garage.toml
    SERVER_ADMIN_ENDPOINT="http://storage:3903"
    SERVER_ADMIN_TOKEN="$GARAGE_ADMIN_TOKEN"
    ;;
  seaweedfs)
    WORKROAM_ACCESS_KEY="${WORKROAM_ACCESS_KEY:-$(key_id)}"
    WORKROAM_SECRET_KEY="${WORKROAM_SECRET_KEY:-$(rand 20)}"
    cat > s3.json <<EOF
{
  "identities": [
    {
      "name": "admin",
      "credentials": [{ "accessKey": "$STORAGE_ADMIN_ACCESS_KEY", "secretKey": "$STORAGE_ADMIN_SECRET_KEY" }],
      "actions": ["Admin", "Read", "List", "Tagging", "Write"]
    },
    {
      "name": "workroam",
      "credentials": [{ "accessKey": "$WORKROAM_ACCESS_KEY", "secretKey": "$WORKROAM_SECRET_KEY" }],
      "actions": ["Read:$BUCKET", "List:$BUCKET", "Tagging:$BUCKET", "Write:$BUCKET"]
    }
  ]
}
EOF
    chmod 644 s3.json
    ;;
esac

if [ -n "$TUNNEL" ]; then SITE="http://$DOMAIN"; else SITE="$DOMAIN"; fi
cat > .env <<EOF
# Generated by Workroam install.sh. Keep this file private: it holds the
# storage admin key, which only the setup wizard needs (once).
COMPOSE_PROJECT_NAME=workroam
COMPOSE_FILE=$compose_file
WORKROAM_STORAGE=$STORAGE
WORKROAM_DOMAIN=$DOMAIN
WORKROAM_SITE=$SITE
WORKROAM_PUBLIC_URL=https://$DOMAIN
STORAGE_UPSTREAM=$UPSTREAM
STORAGE_ADMIN_ACCESS_KEY=$STORAGE_ADMIN_ACCESS_KEY
STORAGE_ADMIN_SECRET_KEY=$STORAGE_ADMIN_SECRET_KEY
SERVER_ADMIN_ACCESS_KEY=$SERVER_ADMIN_ACCESS_KEY
SERVER_ADMIN_SECRET_KEY=$SERVER_ADMIN_SECRET_KEY
SERVER_ADMIN_ENDPOINT=$SERVER_ADMIN_ENDPOINT
SERVER_ADMIN_TOKEN=$SERVER_ADMIN_TOKEN
WORKROAM_NTFY=$NTFY
CF_TUNNEL_TOKEN=$TUNNEL
GARAGE_RPC_SECRET=${GARAGE_RPC_SECRET:-}
GARAGE_ADMIN_TOKEN=${GARAGE_ADMIN_TOKEN:-}
WORKROAM_ACCESS_KEY=${WORKROAM_ACCESS_KEY:-}
WORKROAM_SECRET_KEY=${WORKROAM_SECRET_KEY:-}
GITEA_ADMIN_PASSWORD=${GITEA_ADMIN_PASSWORD:-}
EOF
chmod 600 .env

if [ "$START" = 0 ]; then
  say "Files written; start with: cd $DIR && docker compose up -d"
  exit 0
fi

say "Starting $STORAGE and Caddy"
docker compose pull -q --ignore-pull-failures || true
docker compose up -d --remove-orphans

wait_for() { # wait_for <description> <command...>
  what="$1"; shift
  i=0
  until "$@" >/dev/null 2>&1; do
    i=$((i + 1))
    [ "$i" -lt 60 ] || die "$what did not come up; see: cd $DIR && docker compose logs storage"
    sleep 2
  done
}

case "$STORAGE" in
  garage)
    G() { docker compose exec -T -e RUST_LOG=warn storage /garage "$@"; }
    wait_for "Garage" G status
    if ! G layout show 2>/dev/null | grep -q "Current cluster layout version: [1-9]"; then
      say "Creating the single-node Garage layout"
      node="$(G node id -q | cut -d@ -f1)"
      G layout assign -z dc1 -c 100G "$node" >/dev/null
      G layout apply --version 1 >/dev/null
    fi
    if [ -z "${WORKROAM_ACCESS_KEY:-}" ]; then
      say "Creating bucket $BUCKET and a key limited to it"
      G bucket create "$BUCKET" >/dev/null 2>&1 || true
      out="$(G key create "workroam-$BUCKET")"
      WORKROAM_ACCESS_KEY="$(printf '%s\n' "$out" | awk -F': *' '/^Key ID/ {print $2; exit}')"
      WORKROAM_SECRET_KEY="$(printf '%s\n' "$out" | awk -F': *' '/^Secret key/ {print $2; exit}')"
      [ -n "$WORKROAM_ACCESS_KEY" ] && [ -n "$WORKROAM_SECRET_KEY" ] \
        || die "could not read the new Garage key; run: docker compose exec storage /garage key list"
      G bucket allow --read --write --owner "$BUCKET" --key "$WORKROAM_ACCESS_KEY" >/dev/null
      sed -i.bak -e "s/^WORKROAM_ACCESS_KEY=.*/WORKROAM_ACCESS_KEY=$WORKROAM_ACCESS_KEY/" \
                 -e "s/^WORKROAM_SECRET_KEY=.*/WORKROAM_SECRET_KEY=$WORKROAM_SECRET_KEY/" .env
      rm -f .env.bak
    fi
    ;;
  seaweedfs)
    W() { docker compose exec -T storage sh -c "echo '$1' | weed shell -master=localhost:9333"; }
    wait_for "SeaweedFS" W "cluster.check"
    say "Creating bucket $BUCKET"
    W "s3.bucket.create -name $BUCKET" >/dev/null 2>&1 || true
    ;;
esac

if [ "$WITH_GITEA" = 1 ] && [ -z "${GITEA_ADMIN_PASSWORD:-}" ]; then
  say "Creating the Gitea admin user"
  GITEA_ADMIN_PASSWORD="$(rand 12)"
  wait_for "Gitea" docker compose exec -T -u git gitea gitea admin user list
  docker compose exec -T -u git gitea gitea admin user create --admin --username workroam \
    --password "$GITEA_ADMIN_PASSWORD" --email "workroam@$DOMAIN" --must-change-password=false >/dev/null
  sed -i.bak "s/^GITEA_ADMIN_PASSWORD=.*/GITEA_ADMIN_PASSWORD=$GITEA_ADMIN_PASSWORD/" .env
  rm -f .env.bak
fi

# ---- what to enter in the setup wizard ----------------------------------

ENDPOINT="https://$DOMAIN"
echo
say "Workroam storage is running at $ENDPOINT"
echo
case "$STORAGE" in
  rustfs)
    cat <<EOF
In the Workroam setup wizard (desktop app, or \`workroam setup\`):
  Storage          Self-hosted (install.sh)
  Endpoint         $ENDPOINT
  Access key       $STORAGE_ADMIN_ACCESS_KEY
  Secret key       $STORAGE_ADMIN_SECRET_KEY
  Key              "Let Workroam create a dedicated key" (recommended): the
                   admin key above is used once and is not kept on the computer.

  workroam setup --new --preset rustfs --endpoint $ENDPOINT \\
    --access-key $STORAGE_ADMIN_ACCESS_KEY --secret-key-stdin --key-mode create
EOF
    ;;
  garage|seaweedfs)
    cat <<EOF
In the Workroam setup wizard (desktop app, or \`workroam setup\`):
  Storage          Other S3 compatible
  Endpoint         $ENDPOINT
  Region           us-east-1
  Access key       $WORKROAM_ACCESS_KEY
  Secret key       $WORKROAM_SECRET_KEY
  Bucket           $BUCKET (the key can only reach this bucket)
  Key              "Use this key"

  workroam setup --new --preset other --endpoint $ENDPOINT --region us-east-1 \\
    --access-key $WORKROAM_ACCESS_KEY --secret-key-stdin --key-mode current --bucket $BUCKET
EOF
    ;;
esac
if [ "$WITH_SERVER" = 1 ]; then
  cat <<EOF

workroam-server:  $ENDPOINT   (web reader: $ENDPOINT/web/)
Bind it to one of your computers once, then other computers can join with a
6-digit code and cloud agents can connect:
  1. workroam agent connect <agent>        (prints a bind code wrb1:...)
  2. cd $DIR && docker compose run --rm workroam-server --bind '<code>' --status \\
       && docker compose restart workroam-server
  3. workroam agent connect <agent> --relay $ENDPOINT
EOF
fi
if [ "$WITH_GITEA" = 1 ]; then
  cat <<EOF

Gitea:            $ENDPOINT/git/   user workroam, password $GITEA_ADMIN_PASSWORD
EOF
fi
if [ -n "$TUNNEL" ]; then
  cat <<EOF

Cloudflare Tunnel: in the Zero Trust dashboard, add the public hostname
  $DOMAIN -> http://caddy:80 to this tunnel.
EOF
fi
cat <<EOF

The keys are saved in $DIR/.env (readable only by you).
Docs: https://github.com/$REPO/blob/main/deploy/README.md
EOF
