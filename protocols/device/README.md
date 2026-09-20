# protocols/device — discovery + trust + transfer contract (v0.3, Phase 9b)

Ports: 53317/53318 FastSwap, 53327/53328 Eco. Heartbeat 30s, timeout 120s.
Discovery: subnet-scan of local /24s (skip self/loopback/link-local), 32-way
probes of `GET /api/ecosystem/v1/info`, 120s prune. No mDNS yet.

Identity: stable id + secret (`pkg/ecosystem/identity.json`, atomic writes);
wire carries the SHA-256 fingerprint only. Trust pins in
`trusted_devices.json`; mismatch is a hard MITM error (re-pair).
Pairing: 6-digit OTP, 120s TTL, 3 attempts then burn.

Transport: TLS proxy (rcgen self-signed, ring-only rustls) in front of the
plain-HTTP router. Clients pin the cert fingerprint via `require_pin`
BEFORE connecting; `danger_accept_invalid_certs` is safe only behind that
gate. HTTP routes (same behind TLS):
`GET info`, `POST prepare/confirm/deny`, `POST upload/:session/:file?token=`,
`POST complete`, `GET events/:session` (SSE progress stream, keep-alive).

Envelope: `EcoMessage{version,msg_type,sender_id,sender_name,payload,timestamp_ms,signature}`
HMAC-SHA256 over (sender + timestamp + payload). Reject bad sig, >5min skew,
version mismatch.

Transfers (`/api/igris/v1/share/`):
`POST prepare{from_device,files[]} -> {session_id,tokens{file:token}}`
`POST confirm{session_id} -> {status:ready}` / `POST deny -> {status:denied}`
`POST upload/:session/:file?token=` (401 bad token, 409 wrong state, 404 unknown)
`POST complete{session_id,file}` (checksum-verified, fails closed)
Chunks: 1MB bitsets, `missing_chunks()` = resume vector. TTL: 120s pending, 600s active.

Sync: generic LWW `(updated_ms, device_id)` + tombstones + deterministic
tie-break + `delta_since` exchange; memory/task adapters ride on top.
