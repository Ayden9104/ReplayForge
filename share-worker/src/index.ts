import { AwsClient } from "aws4fetch";

export interface Env {
  CLIPS: R2Bucket;
  PUBLIC_BASE_URL: string;
  R2_BUCKET_NAME: string;
  R2_ACCESS_KEY_ID: string;
  R2_SECRET_ACCESS_KEY: string;
  R2_ACCOUNT_ID: string;
}

const MAX_BYTES = 500 * 1024 * 1024;
const PRESIGN_EXPIRES_SECS = 600;
const UA_PREFIX = "ReplayForge/";

export default {
  async fetch(request: Request, env: Env): Promise<Response> {
    const url = new URL(request.url);
    const path = url.pathname.replace(/\/+$/, "") || "/";

    try {
      if (request.method === "OPTIONS") {
        return cors(new Response(null, { status: 204 }));
      }

      if (request.method === "POST" && path === "/v1/upload") {
        return cors(await handleUploadInit(request, env));
      }

      const completeMatch = path.match(/^\/v1\/upload\/([a-f0-9-]{36})\/complete$/);
      if (request.method === "POST" && completeMatch) {
        return cors(await handleUploadComplete(completeMatch[1], env));
      }

      const clipMatch = path.match(/^\/c\/([a-f0-9-]{36})$/);
      if (request.method === "GET" && clipMatch) {
        return await handleClipGet(clipMatch[1], request, env);
      }

      if (request.method === "GET" && path === "/") {
        return cors(
          json({
            service: "replayforge-share",
            endpoints: ["POST /v1/upload", "POST /v1/upload/:id/complete", "GET /c/:id"],
          }),
        );
      }

      return cors(json({ error: "not found" }, 404));
    } catch (error) {
      const message = error instanceof Error ? error.message : "internal error";
      return cors(json({ error: message }, 500));
    }
  },
} satisfies ExportedHandler<Env>;

async function handleUploadInit(request: Request, env: Env): Promise<Response> {
  const ua = request.headers.get("User-Agent") || "";
  if (!ua.startsWith(UA_PREFIX)) {
    return json({ error: "forbidden: ReplayForge User-Agent required" }, 403);
  }

  if (!(await allowRate(request))) {
    return json({ error: "rate limited — try again shortly" }, 429);
  }

  requireSecrets(env);

  let body: { size?: number; filename?: string } = {};
  try {
    body = (await request.json()) as { size?: number; filename?: string };
  } catch {
    return json({ error: "expected JSON body { size, filename }" }, 400);
  }

  const size = Number(body.size ?? 0);
  if (!Number.isFinite(size) || size <= 0) {
    return json({ error: "size must be a positive number" }, 400);
  }
  if (size > MAX_BYTES) {
    return json(
      { error: `file too large; max is ${MAX_BYTES} bytes (~500 MB)` },
      413,
    );
  }

  const id = crypto.randomUUID();
  const key = objectKey(id);
  const uploadUrl = await presignPut(env, key);
  const base = env.PUBLIC_BASE_URL.replace(/\/+$/, "");
  const shareUrl = `${base}/c/${id}`;

  return json({ id, uploadUrl, shareUrl });
}

async function handleUploadComplete(id: string, env: Env): Promise<Response> {
  const obj = await env.CLIPS.head(objectKey(id));
  if (!obj) {
    return json({ error: "upload not found — PUT may have failed" }, 404);
  }
  return json({
    ok: true,
    id,
    size: obj.size,
    shareUrl: `${env.PUBLIC_BASE_URL.replace(/\/+$/, "")}/c/${id}`,
  });
}

async function handleClipGet(
  id: string,
  request: Request,
  env: Env,
): Promise<Response> {
  if (wantsRaw(request)) {
    const obj = await env.CLIPS.get(objectKey(id));
    if (!obj) {
      return htmlResponse(notFoundHtml(), 404);
    }
    const headers = new Headers();
    obj.writeHttpMetadata(headers);
    headers.set("Content-Type", "video/mp4");
    headers.set("Accept-Ranges", "bytes");
    headers.set("Cache-Control", "public, max-age=3600");
    headers.set("Access-Control-Allow-Origin", "*");
    if (obj.size) {
      headers.set("Content-Length", String(obj.size));
    }
    return new Response(obj.body, { status: 200, headers });
  }

  const meta = await env.CLIPS.head(objectKey(id));
  if (!meta) {
    return htmlResponse(notFoundHtml(), 404);
  }
  return htmlResponse(playerHtml(id), 200);
}

/** Raw MP4 when ?raw=1, or Accept prefers video without HTML. */
function wantsRaw(request: Request): boolean {
  const url = new URL(request.url);
  if (url.searchParams.get("raw") === "1") {
    return true;
  }
  const accept = (request.headers.get("Accept") || "").toLowerCase();
  if (!accept || accept.includes("*/*")) {
    return false;
  }
  const wantsHtml = accept.includes("text/html");
  const wantsVideo = accept.includes("video/");
  return wantsVideo && !wantsHtml;
}

function htmlResponse(body: string, status: number): Response {
  return new Response(body, {
    status,
    headers: {
      "Content-Type": "text/html; charset=utf-8",
      "Cache-Control": "public, max-age=300",
    },
  });
}

function brandMarkSvg(): string {
  // Compact mark from assets/replayforge.svg (teal play-in-ring on dark tile).
  return `<svg class="mark" xmlns="http://www.w3.org/2000/svg" viewBox="0 0 128 128" role="img" aria-hidden="true">
  <rect width="128" height="128" rx="28" fill="#12212b"/>
  <circle cx="64" cy="64" r="36" fill="none" stroke="#2eccb0" stroke-width="10"/>
  <polygon points="56,44 88,64 56,84" fill="#2eccb0"/>
</svg>`;
}

function sharedPageCss(): string {
  return `
    :root {
      --bg: #191919;
      --bg-glow: #1e2636;
      --accent: #4682dc;
      --accent-hover: #5b93e8;
      --muted: #8c8c8c;
      --text: #e8e8e8;
      --video-bg: #0a0a0a;
    }
    * { box-sizing: border-box; }
    body {
      margin: 0;
      min-height: 100vh;
      background:
        radial-gradient(ellipse 90% 55% at 50% -10%, var(--bg-glow) 0%, transparent 70%),
        var(--bg);
      color: var(--text);
      font-family: "DM Sans", system-ui, sans-serif;
      display: flex;
      flex-direction: column;
      align-items: center;
      padding: 20px 16px 32px;
    }
    .brand-bar {
      display: flex;
      align-items: center;
      gap: 12px;
      margin: 0 0 6px;
    }
    .mark {
      width: 36px;
      height: 36px;
      flex-shrink: 0;
      border-radius: 8px;
    }
    .brand {
      font-size: clamp(1.35rem, 3.5vw, 1.75rem);
      font-weight: 700;
      letter-spacing: -0.03em;
      color: var(--accent);
      margin: 0;
    }
    .tag {
      margin: 0 0 18px;
      color: var(--muted);
      font-size: 0.9rem;
      text-align: center;
      font-weight: 500;
    }
    .stage {
      width: min(1100px, 100%);
      animation: rise 280ms ease-out both;
    }
    @keyframes rise {
      from { opacity: 0; transform: translateY(12px); }
      to { opacity: 1; transform: translateY(0); }
    }
    video {
      display: block;
      width: 100%;
      max-height: min(78vh, 820px);
      background: var(--video-bg);
      border-radius: 8px;
      box-shadow: 0 18px 48px rgba(0, 0, 0, 0.45);
    }
    .actions {
      margin-top: 18px;
      text-align: center;
    }
    .btn {
      display: inline-block;
      padding: 10px 18px;
      border-radius: 8px;
      background: var(--accent);
      color: #fff;
      text-decoration: none;
      font-size: 0.95rem;
      font-weight: 600;
      letter-spacing: -0.01em;
      transition: background 120ms ease;
    }
    .btn:hover { background: var(--accent-hover); }
    .center {
      flex: 1;
      display: flex;
      flex-direction: column;
      align-items: center;
      justify-content: center;
      text-align: center;
      width: min(28rem, 100%);
      animation: rise 280ms ease-out both;
    }
    .center p {
      color: var(--muted);
      margin: 10px 0 0;
      line-height: 1.5;
      font-size: 1.05rem;
    }
`;
}

function playerHtml(id: string): string {
  const rawSrc = `/c/${id}?raw=1`;
  return `<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="utf-8" />
  <meta name="viewport" content="width=device-width, initial-scale=1" />
  <title>ReplayForge — Shared clip</title>
  <link rel="preconnect" href="https://fonts.googleapis.com" />
  <link rel="preconnect" href="https://fonts.gstatic.com" crossorigin />
  <link href="https://fonts.googleapis.com/css2?family=DM+Sans:opsz,wght@9..40,500;9..40,600;9..40,700&display=swap" rel="stylesheet" />
  <style>${sharedPageCss()}</style>
</head>
<body>
  <div class="brand-bar">
    ${brandMarkSvg()}
    <h1 class="brand">ReplayForge</h1>
  </div>
  <p class="tag">Shared clip · Expires in about 7 days</p>
  <div class="stage">
    <video controls playsinline preload="metadata" src="${rawSrc}"></video>
    <div class="actions">
      <a class="btn" href="${rawSrc}" download>Download MP4</a>
    </div>
  </div>
</body>
</html>`;
}

function notFoundHtml(): string {
  return `<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="utf-8" />
  <meta name="viewport" content="width=device-width, initial-scale=1" />
  <title>ReplayForge — Clip not found</title>
  <link rel="preconnect" href="https://fonts.googleapis.com" />
  <link rel="preconnect" href="https://fonts.gstatic.com" crossorigin />
  <link href="https://fonts.googleapis.com/css2?family=DM+Sans:opsz,wght@9..40,500;9..40,600;9..40,700&display=swap" rel="stylesheet" />
  <style>${sharedPageCss()}</style>
</head>
<body>
  <div class="center">
    <div class="brand-bar">
      ${brandMarkSvg()}
      <h1 class="brand">ReplayForge</h1>
    </div>
    <p>Clip not found or expired.</p>
  </div>
</body>
</html>`;
}

async function presignPut(env: Env, key: string): Promise<string> {
  const client = new AwsClient({
    accessKeyId: env.R2_ACCESS_KEY_ID,
    secretAccessKey: env.R2_SECRET_ACCESS_KEY,
    service: "s3",
    region: "auto",
  });

  const url = `https://${env.R2_ACCOUNT_ID}.r2.cloudflarestorage.com/${env.R2_BUCKET_NAME}/${key}?X-Amz-Expires=${PRESIGN_EXPIRES_SECS}`;
  const signed = await client.sign(
    new Request(url, {
      method: "PUT",
      headers: { "Content-Type": "video/mp4" },
    }),
    { aws: { signQuery: true } },
  );
  return signed.url;
}

function objectKey(id: string): string {
  return `clips/${id}.mp4`;
}

function requireSecrets(env: Env): void {
  for (const key of [
    "R2_ACCESS_KEY_ID",
    "R2_SECRET_ACCESS_KEY",
    "R2_ACCOUNT_ID",
    "R2_BUCKET_NAME",
    "PUBLIC_BASE_URL",
  ] as const) {
    if (!env[key]) {
      throw new Error(`missing env/secret: ${key}`);
    }
  }
}

/** Soft per-IP rate limit via Cache API (~10 upload inits / minute). */
async function allowRate(request: Request): Promise<boolean> {
  const ip =
    request.headers.get("CF-Connecting-IP") ||
    request.headers.get("X-Forwarded-For")?.split(",")[0]?.trim() ||
    "unknown";
  const minute = Math.floor(Date.now() / 60_000);
  const cacheKey = new Request(
    `https://replayforge-share.rate/init/${ip}/${minute}`,
  );
  const cache = caches.default;
  const existing = await cache.match(cacheKey);
  const count = existing ? Number(await existing.text()) : 0;
  if (count >= 10) {
    return false;
  }
  await cache.put(
    cacheKey,
    new Response(String(count + 1), {
      headers: { "Cache-Control": "max-age=120" },
    }),
  );
  return true;
}

function json(data: unknown, status = 200): Response {
  return new Response(JSON.stringify(data), {
    status,
    headers: { "Content-Type": "application/json; charset=utf-8" },
  });
}

function cors(response: Response): Response {
  const headers = new Headers(response.headers);
  headers.set("Access-Control-Allow-Origin", "*");
  headers.set("Access-Control-Allow-Methods", "GET, POST, OPTIONS");
  headers.set("Access-Control-Allow-Headers", "Content-Type, User-Agent");
  return new Response(response.body, { status: response.status, headers });
}
