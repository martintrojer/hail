# Reverse proxy for hail

## When you need this

Use a reverse proxy when you are not using Cloudflare Tunnel and you want to
expose hail-api publicly with normal HTTPS on port 443.

The proxy terminates TLS and forwards traffic to `hail-api` on the internal
Compose network. Keep Stalwart SMTP ports separate; this document is only for
the web UI, `/api`, and `/api/ws`.

## Caddy example

Minimal Caddyfile with automatic HTTPS via Let's Encrypt:

```caddyfile
mail.example.com {
  reverse_proxy hail-api:8080
}
```

Caddy forwards WebSocket upgrade headers automatically. If you want explicit
forwarded headers, use:

```caddyfile
mail.example.com {
  reverse_proxy hail-api:8080 {
    header_up X-Forwarded-Proto {scheme}
    header_up X-Forwarded-For {remote_host}
  }
}
```

## Caddy Compose overlay

Save as `deploy/docker-compose.caddy.yml`:

```yaml
services:
  caddy:
    image: caddy:2
    restart: unless-stopped
    ports:
      - "80:80"
      - "443:443"
    volumes:
      - ./Caddyfile:/etc/caddy/Caddyfile:ro
      - caddy-data:/data
      - caddy-config:/config
    depends_on:
      - hail-api
    networks:
      - default

volumes:
  caddy-data:
  caddy-config:
```

Run it with the base stack:

```bash
podman compose \
  -f deploy/docker-compose.yml \
  -f deploy/docker-compose.caddy.yml \
  up -d
```

Docker equivalent: replace `podman compose` with `docker compose`.

## Traefik example

Traefik is usually labels-based. Add labels to the `hail-api` service and run a
Traefik service on the same Compose network:

```yaml
services:
  hail-api:
    labels:
      - traefik.enable=true
      - traefik.http.routers.hail.rule=Host(`mail.example.com`)
      - traefik.http.routers.hail.entrypoints=websecure
      - traefik.http.routers.hail.tls.certresolver=letsencrypt
      - traefik.http.services.hail.loadbalancer.server.port=8080

  traefik:
    image: traefik:v3.1
    restart: unless-stopped
    command:
      - --providers.docker=true
      - --providers.docker.exposedbydefault=false
      - --entrypoints.web.address=:80
      - --entrypoints.websecure.address=:443
      - --certificatesresolvers.letsencrypt.acme.email=you@example.com
      - --certificatesresolvers.letsencrypt.acme.storage=/letsencrypt/acme.json
      - --certificatesresolvers.letsencrypt.acme.httpchallenge=true
      - --certificatesresolvers.letsencrypt.acme.httpchallenge.entrypoint=web
    ports:
      - "80:80"
      - "443:443"
    volumes:
      - /var/run/docker.sock:/var/run/docker.sock:ro
      - traefik-letsencrypt:/letsencrypt

volumes:
  traefik-letsencrypt:
```

## Forwarded headers

Forward these headers from the proxy to hail-api:

- `X-Forwarded-Proto`: original request scheme, usually `https`.
- `X-Forwarded-For`: original client IP chain.

hail-api trusts forwarded headers only when it is configured to run behind a
known proxy. Do not expose `hail-api:8080` directly to the internet while also
trusting arbitrary forwarded headers.

## WebSocket gotcha

The app uses `/api/ws`. The proxy must pass HTTP upgrade headers.

Caddy usually needs no special configuration. Explicit form:

```caddyfile
@websocket {
  header Connection *Upgrade*
  header Upgrade websocket
}
reverse_proxy @websocket hail-api:8080
reverse_proxy hail-api:8080
```

Traefik handles WebSockets automatically for HTTP routers. If you add custom
middlewares, do not strip these headers:

```yaml
labels:
  - traefik.http.routers.hail.rule=Host(`mail.example.com`)
  - traefik.http.routers.hail.entrypoints=websecure
  - traefik.http.routers.hail.tls.certresolver=letsencrypt
  - traefik.http.services.hail.loadbalancer.server.port=8080
```

After changing proxy config, reload the page and confirm the browser dev tools
show `/api/ws` with status `101 Switching Protocols`.
