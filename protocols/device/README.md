# protocols/device — discovery + trust + transfer contract (v0.2, Phase 9a)

Ports: 53317/53318 FastSwap, 53327/53328 Eco. Heartbeat 30s, timeout 120s.
Discovery: subnet-scan of local /24s (skip self/loopback/link-local), 32-way
probes of `GET /api/ecosystem/v1/info`, 120s prune. No mDNS yet.

Identity: stable id + secret; wire carries the SHA-256 fingerprint only.
Trust: TOFU pin on first sight; mismatch is a hard MITM error (re-pair).
Pairing: 6-digit OTP, 120s TTL, 3 attempts then burn.

Envelope: `EcoMessage{version,msg_type,sender_id,sender_name,payload,timestamp_ms,signature}`
HMAC-SHA256 over (sender + timestamp + payload). Reject bad sig, >5min skew,
version mismatch.

Transfers (`/api/igris/v1/share/`):
`POST prepare{from_device,files[]} -> {session_id,tokens{file:token}}`
`POST confirm{session_id} -> {status:ready}` / `POST deny -> {status:denied}`
`POST upload/:session/:file?token=` (401 bad token, 409 wrong state, 404 unknown)
`POST complete{session_id,file}` (checksum-verified, fails closed)
Chunks: 1MB bitsets, `missing_chunks()` = resume vector. TTL: 120s pending, 600s active.

9b: TLS proxy (self-signed + real pin gate), trust/identity file persistence,
memory/task LWW sync, SSE progress.
