# Back up and restore hail

Back up two things:

1. `hail.db` in `/var/lib/hail`, the SQLite sidecar used by hail-api and
   hail-worker.
2. The Stalwart data volume, especially the filesystem blob store with raw
   messages and attachments.

Litestream protects `hail.db`. A scheduled tarball protects `stalwart-data`.

## SQLite and Litestream

hail uses SQLite in WAL mode. That is enabled by `hail-db`; see
`docs/design.md` §5, DD-7. Operators do not need to run SQLite pragmas.

Add a Litestream sidecar:

```yaml
services:
  litestream:
    image: litestream/litestream:0.3
    restart: unless-stopped
    command: ["replicate", "-config", "/etc/litestream.yml"]
    environment:
      AWS_ACCESS_KEY_ID: ${LITESTREAM_ACCESS_KEY_ID:-}
      AWS_SECRET_ACCESS_KEY: ${LITESTREAM_SECRET_ACCESS_KEY:-}
      AWS_REGION: ${LITESTREAM_REGION:-us-east-1}
    volumes:
      - hail-data:/var/lib/hail
      - ./litestream.yml:/etc/litestream.yml:ro
    depends_on:
      - hail-api
```

### S3 replica

```yaml
dbs:
  - path: /var/lib/hail/hail.db
    replicas:
      - type: s3
        bucket: my-hail-backups
        path: hail.db
        region: us-east-1
```

`.env`:

```dotenv
LITESTREAM_ACCESS_KEY_ID=AKIA...
LITESTREAM_SECRET_ACCESS_KEY=...
LITESTREAM_REGION=us-east-1
```

### Cloudflare R2 replica

R2 uses the S3 API with a custom endpoint:

```yaml
dbs:
  - path: /var/lib/hail/hail.db
    replicas:
      - type: s3
        bucket: hail-backups
        path: hail.db
        endpoint: https://ACCOUNT_ID.r2.cloudflarestorage.com
        region: auto
        force-path-style: true
```

### Local NFS replica

Mount NFS on the host and bind it into Litestream:

```bash
sudo mkdir -p /mnt/backup/hail
sudo mount nfs.example.com:/exports/hail /mnt/backup/hail
```

```yaml
services:
  litestream:
    volumes:
      - hail-data:/var/lib/hail
      - /mnt/backup/hail:/backup
      - ./litestream.yml:/etc/litestream.yml:ro
```

```yaml
dbs:
  - path: /var/lib/hail/hail.db
    replicas:
      - type: file
        path: /backup/hail.db
```

## Stalwart volume backup

Archive the `stalwart-data` volume on a schedule:

```bash
#!/usr/bin/env bash
set -euo pipefail
stamp=$(date -u +%Y%m%dT%H%M%SZ)
mkdir -p /srv/backups/hail
podman run --rm \
  -v stalwart-data:/data:ro \
  -v /srv/backups/hail:/backup \
  docker.io/library/alpine:3.20 \
  tar -C /data -czf /backup/stalwart-data-${stamp}.tar.gz .
```

Cron, nightly at 03:17:

```cron
17 3 * * * /usr/local/sbin/backup-stalwart-data.sh >/var/log/hail-stalwart-backup.log 2>&1
```

For Docker, replace `podman run` with `docker run`.

## Restore procedure

1. Stop containers:

   ```bash
   podman compose -f deploy/docker-compose.yml down
   # or: docker compose -f deploy/docker-compose.yml down
   ```

2. Restore `hail.db` with Litestream:

   ```bash
   mkdir -p ./restore/hail
   litestream restore -if-replica-exists \
     -o ./restore/hail/hail.db \
     s3://my-hail-backups/hail.db
   podman run --rm -v hail-data:/data -v "$PWD/restore/hail:/restore:ro" \
     docker.io/library/alpine:3.20 sh -c 'cp /restore/hail.db /data/hail.db'
   ```

3. Extract the Stalwart tarball into a fresh volume:

   ```bash
   podman volume rm stalwart-data
   podman volume create stalwart-data
   podman run --rm -v stalwart-data:/data -v /srv/backups/hail:/backup:ro \
     docker.io/library/alpine:3.20 \
     tar -C /data -xzf /backup/stalwart-data-YYYYMMDDTHHMMSSZ.tar.gz
   ```

4. Start containers and check readiness:

   ```bash
   podman compose -f deploy/docker-compose.yml up -d
   curl -i http://127.0.0.1:8080/readyz
   ```

5. Sign in and confirm recent mail and Screener state are present.

## Restore drill frequency

Run a tested restore at least quarterly, into a separate host or throwaway
volumes. A backup that has never been restored is only a guess.
