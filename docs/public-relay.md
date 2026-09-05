# Public Relay deployment

The Relay accepts an outbound Host WSS connection and browser WSS connections. The Host computer needs no public inbound port.

## Token

Set the same high-entropy registration token on the Relay and Host. Pass only the environment variable name to the binaries; do not place the token itself in command history.

```bash
export AGENT_REMOTE_RELAY_TOKEN='replace-with-a-random-deployment-token'
```

This token authorizes Host registration. Browser device authorization still happens at the Host with pair/device credentials.

## Direct rustls

```bash
./agent-remote-relay serve \
  --listen 0.0.0.0:8443 \
  --tls-cert /etc/letsencrypt/live/relay.example.com/fullchain.pem \
  --tls-key /etc/letsencrypt/live/relay.example.com/privkey.pem \
  --web-dir ./web
```

Allow inbound TCP 8443 (or the chosen HTTPS port) on the Relay server. The Host needs outbound HTTPS/WSS access only.

## Caddy

Run the Relay on loopback with its explicit reverse-proxy mode:

```bash
./agent-remote-relay serve \
  --listen 127.0.0.1:8443 \
  --behind-proxy \
  --web-dir ./web
```

```caddyfile
relay.example.com {
    reverse_proxy 127.0.0.1:8443
}
```

Caddy terminates TLS and forwards WebSocket upgrades automatically. Only ports 80/443 need public firewall access; 8443 remains loopback-only.

## Nginx

```nginx
server {
    listen 443 ssl;
    server_name relay.example.com;

    ssl_certificate     /etc/letsencrypt/live/relay.example.com/fullchain.pem;
    ssl_certificate_key /etc/letsencrypt/live/relay.example.com/privkey.pem;

    location / {
        proxy_pass http://127.0.0.1:8443;
        proxy_http_version 1.1;
        proxy_set_header Upgrade $http_upgrade;
        proxy_set_header Connection "upgrade";
        proxy_set_header Host $host;
    }
}
```

Use the same loopback `--behind-proxy` Relay command as the Caddy example.

## Connect the Host

```powershell
$env:AGENT_REMOTE_RELAY_TOKEN = 'replace-with-the-same-token'
agent-remote-host.exe serve `
  --relay-url wss://relay.example.com/host `
  --relay-token-env AGENT_REMOTE_RELAY_TOKEN `
  --web-root dist\web
```

Generate a Relay pairing link:

```powershell
agent-remote-host.exe pair --relay --base-url https://relay.example.com
```

The browser connects to `/client/<host-id>`. The Relay opens a logical connection over the existing Host tunnel and forwards the same application CBOR used by direct mode.

The Host tunnel and the Relay's browser connections send a standard WebSocket Ping every 30 seconds so idle connections remain active through reverse proxies. Browsers respond with Pong automatically; no application message or conversation activity is generated.

## Offline behavior

When the Host tunnel disconnects, connected browsers receive an explicit offline status and their logical connections close. The Host reconnects with a small capped delay. Browsers reconnect and request a complete Snapshot. Commands are not stored or queued while the Host is offline.

The Relay also sends Ping to each Host every 30 seconds. If no Host frame or Pong arrives for 90 seconds, it removes that tunnel and notifies its clients that the Host is offline. This frees the Host ID for reconnection when a broken network path leaves the old WebSocket apparently open.

## Privacy boundary

The Relay keeps only an in-memory online Host/client routing map. It does not persist projects, messages, images, device tokens, or offline jobs. It is nevertheless a trusted transport endpoint that can observe forwarded payloads in memory because v0.1 does not implement application-layer E2EE. HTTPS/WSS protects traffic on the network; it is not an end-to-end-encryption claim.
