# ReplayForge share worker (Cloudflare R2)

Medal-style clip links: the desktop app uploads to R2 via a short-lived presigned URL, then friends open `https://<worker>/c/<id>`.

**Production Worker (baked into the app):**  
`https://replayforge-share.holdup6699.workers.dev`

End users do not need this guide. This file is for maintainers / self-hosting.

## What you need

- Cloudflare account
- Node.js 18+ (for Wrangler)
- An R2 bucket (e.g. `replayforge-clips`)
- R2 API token with Object Read & Write (Access Key ID + Secret Access Key)

## Setup

```bash
cd share-worker
npm install
```

1. Create R2 bucket `replayforge-clips` in the Cloudflare dashboard (or rename and match `wrangler.toml`).
2. Create an R2 API token: **R2 → Manage R2 API Tokens → Create API token**.
3. Edit [`wrangler.toml`](wrangler.toml):
   - `bucket_name` / `R2_BUCKET_NAME`
   - `PUBLIC_BASE_URL` (set to your worker URL after first deploy, then redeploy)
4. Put secrets:

```bash
npx wrangler secret put R2_ACCESS_KEY_ID
npx wrangler secret put R2_SECRET_ACCESS_KEY
npx wrangler secret put R2_ACCOUNT_ID   # Cloudflare account id (dashboard URL / workers overview)
```

5. Deploy:

```bash
npx wrangler deploy
```

6. Set `PUBLIC_BASE_URL` in `wrangler.toml` to the printed `*.workers.dev` URL and deploy again.

7. **Lifecycle (7-day expiry):** R2 → bucket → Settings → Object lifecycle rules → delete objects after **7 days** (prefix `clips/` if offered).

8. In ReplayForge **Settings → Sharing**, paste the worker base URL (no trailing path), e.g. `https://replayforge-share.<account>.workers.dev`.

## API

| Method | Path | Notes |
|--------|------|--------|
| `POST` | `/v1/upload` | Body `{ "size": N, "filename": "x.mp4" }`. Requires `User-Agent: ReplayForge/...`. Returns `{ id, uploadUrl, shareUrl }`. |
| `PUT` | *(presigned R2 URL)* | App uploads raw MP4 with `Content-Type: video/mp4`. |
| `POST` | `/v1/upload/:id/complete` | Confirms object exists. |
| `GET` | `/c/:id` | Streams the clip. |

Max size: **500 MB**. Soft rate limit: ~10 upload inits per IP per minute.

## Cost note

R2 storage is cheap; **egress** through the Worker when viewers watch clips is the main variable cost. Start with short retention (7 days) and monitor the Cloudflare bill.

## Local dev

```bash
npx wrangler dev
```

You still need real R2 credentials/secrets for presigned uploads to work against a remote bucket.
