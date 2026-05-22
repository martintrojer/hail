# Upgrade hail

This guide covers routine container upgrades, schema migrations, rollback, and
Stalwart version handling.

## Versioning policy

hail follows semantic versioning:

- Patch releases fix bugs and should be safe to apply routinely.
- Minor releases add features and may include database migrations.
- Major releases may require operator action.

While hail is `v0.x`, breaking changes are possible in minor releases. Read the
release notes before upgrading any production mailbox.

## Routine upgrade

Make sure backups are current first. At minimum, confirm Litestream is
replicating `hail.db` and that the latest Stalwart volume tarball exists.

Pull and restart with Podman:

```bash
podman compose -f deploy/docker-compose.yml pull
podman compose -f deploy/docker-compose.yml up -d
podman compose -f deploy/docker-compose.yml ps
```

Docker equivalent:

```bash
docker compose -f deploy/docker-compose.yml pull
docker compose -f deploy/docker-compose.yml up -d
docker compose -f deploy/docker-compose.yml ps
```

Check health and readiness:

```bash
curl -i http://127.0.0.1:8080/healthz
curl -i http://127.0.0.1:8080/readyz
```

Migrations run automatically when `hail-api` starts. Keep `hail-api` and
`hail-worker` on the same hail image tag.

## Schema migrations

hail uses `sqlx::migrate!()` with migration files embedded into the `hail-api`
binary at build time. sqlx creates and maintains the `_sqlx_migrations` table in
`hail.db`.

On startup, `hail-api` applies pending migrations before accepting requests.
`hail-worker` expects the schema version it was compiled against. There is no
operator-side migration command for normal upgrades.

If startup fails during migration:

```bash
podman compose -f deploy/docker-compose.yml logs --tail=200 hail-api
```

Do not edit `_sqlx_migrations` manually. Restore from backup if a migration was
partially applied and the release notes do not give a specific repair command.

## Rollback

Rollback means restoring data and running the old image tag. Do not only change
the image tag after a failed schema migration; the database may already be newer
than the old binary expects.

1. Stop containers:

   ```bash
   podman compose -f deploy/docker-compose.yml down
   ```

2. Restore `hail.db` from Litestream using the procedure in `docs/backup.md`.

3. Restore the matching Stalwart volume tarball if the failed upgrade also
   changed Stalwart or mail storage.

4. Downgrade the hail image tag in `deploy/docker-compose.yml` or `.env`.

5. Start the stack:

   ```bash
   podman compose -f deploy/docker-compose.yml up -d
   curl -i http://127.0.0.1:8080/readyz
   ```

For Docker, replace `podman compose` with `docker compose`.

## Stalwart upgrades

Stalwart has a separate upgrade cycle. hail does not pin Stalwart releases for
you. Pin the Stalwart image in `deploy/docker-compose.yml` instead of tracking
`latest`, for example:

```yaml
services:
  stalwart:
    image: stalwartlabs/stalwart:v0.13.3 # pin; upgrade intentionally
```

Upgrade Stalwart separately from hail when possible:

1. Back up `hail.db` and `stalwart-data`.
2. Read Stalwart release notes.
3. Change only the Stalwart tag.
4. Restart and confirm JMAP login plus `/readyz`.

## Breaking-change announcements

Breaking changes should be announced in `CHANGELOG.md` and release notes before
you upgrade. If `CHANGELOG.md` does not exist yet, track its creation as a
follow-up documentation task before the first tagged release.
