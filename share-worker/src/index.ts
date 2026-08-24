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
const GITHUB_RELEASES = "https://github.com/Ayden9104/ReplayForge/releases/latest";

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
        return cors(await handleUploadComplete(completeMatch[1], request, env));
      }

      const thumbMatch = path.match(/^\/c\/([a-f0-9-]{36})\/thumb\.jpg$/);
      if (request.method === "GET" && thumbMatch) {
        return await handleThumbGet(thumbMatch[1], request, env);
      }

      const clipMatch = path.match(/^\/c\/([a-f0-9-]{36})$/);
      if (request.method === "GET" && clipMatch) {
        return await handleClipGet(clipMatch[1], request, env);
      }

      if (request.method === "GET" && path === "/") {
        return cors(
          json({
            service: "replayforge-share",
            endpoints: [
              "POST /v1/upload",
              "POST /v1/upload/:id/complete",
              "GET /c/:id",
              "GET /c/:id/thumb.jpg",
            ],
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
  // UA prefix is a casual filter only — not real authentication.
  const ua = request.headers.get("User-Agent") || "";
  if (!ua.startsWith(UA_PREFIX)) {
    return json({ error: "forbidden: ReplayForge User-Agent required" }, 403);
  }

  if (!(await allowRate(request, "init", 5))) {
    return json({ error: "rate limited — try again shortly" }, 429);
  }

  requireSecrets(env);
  requireHttpsPublicBase(env);

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
  const uploadUrl = await presignPut(env, objectKey(id), "video/mp4");
  const thumbUploadUrl = await presignPut(env, thumbKey(id), "image/jpeg");
  const base = env.PUBLIC_BASE_URL.replace(/\/+$/, "");
  const shareUrl = `${base}/c/${id}`;

  return json({ id, uploadUrl, thumbUploadUrl, shareUrl });
}

async function handleUploadComplete(
  id: string,
  request: Request,
  env: Env,
): Promise<Response> {
  // UA prefix is a casual filter only — not real authentication.
  const ua = request.headers.get("User-Agent") || "";
  if (!ua.startsWith(UA_PREFIX)) {
    return json({ error: "forbidden: ReplayForge User-Agent required" }, 403);
  }

  if (!(await allowRate(request, "complete", 5))) {
    return json({ error: "rate limited — try again shortly" }, 429);
  }

  requireHttpsPublicBase(env);

  const key = objectKey(id);
  const obj = await env.CLIPS.head(key);
  if (!obj) {
    return json({ error: "upload not found — PUT may have failed" }, 404);
  }
  if (obj.size > MAX_BYTES) {
    await env.CLIPS.delete(key);
    return json(
      { error: `file too large; max is ${MAX_BYTES} bytes (~500 MB)` },
      413,
    );
  }
  return json({
    ok: true,
    id,
    size: obj.size,
    shareUrl: `${env.PUBLIC_BASE_URL.replace(/\/+$/, "")}/c/${id}`,
  });
}

async function handleThumbGet(
  id: string,
  request: Request,
  env: Env,
): Promise<Response> {
  if (!(await allowRate(request, "get", 60))) {
    return json({ error: "rate limited — try again shortly" }, 429);
  }

  const obj = await env.CLIPS.get(thumbKey(id));
  if (!obj) {
    return new Response("Not found", { status: 404 });
  }
  const headers = new Headers();
  obj.writeHttpMetadata(headers);
  headers.set("Content-Type", "image/jpeg");
  headers.set("Cache-Control", "public, max-age=3600");
  headers.set("Access-Control-Allow-Origin", "*");
  if (obj.size) {
    headers.set("Content-Length", String(obj.size));
  }
  return new Response(obj.body, { status: 200, headers });
}

async function handleClipGet(
  id: string,
  request: Request,
  env: Env,
): Promise<Response> {
  if (!(await allowRate(request, "get", 60))) {
    return json({ error: "rate limited — try again shortly" }, 429);
  }

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
  const base = env.PUBLIC_BASE_URL.replace(/\/+$/, "");
  const hasThumb = !!(await env.CLIPS.head(thumbKey(id)));
  const expiryLine = shareExpiryLine(meta.uploaded);
  return htmlResponse(playerHtml(id, base, hasThumb, expiryLine), 200);
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
      padding: max(16px, env(safe-area-inset-top)) 16px max(28px, env(safe-area-inset-bottom));
      transition: background 420ms ease;
    }
    .brand-bar {
      display: flex;
      align-items: center;
      gap: 12px;
      margin: 0 0 6px;
    }
    .brand-bar.egg {
      cursor: pointer;
      user-select: none;
      border-radius: 10px;
      padding: 4px 8px 4px 4px;
      transition: transform 160ms ease, background 200ms ease;
    }
    .brand-bar.egg:hover {
      background: rgba(70, 130, 220, 0.08);
    }
    .brand-bar.egg:active {
      transform: scale(0.98);
    }
    .mark {
      width: 36px;
      height: 36px;
      flex-shrink: 0;
      border-radius: 8px;
      transition: transform 280ms ease;
    }
    .brand {
      font-size: clamp(1.35rem, 3.5vw, 1.75rem);
      font-weight: 700;
      letter-spacing: -0.03em;
      color: var(--accent);
      margin: 0;
      transition: color 280ms ease;
    }
    .tag {
      margin: 0 0 20px;
      color: var(--muted);
      font-size: 0.92rem;
      text-align: center;
      font-weight: 600;
      letter-spacing: 0.01em;
      min-height: 1.35em;
      transition: color 280ms ease, letter-spacing 280ms ease;
    }
    body.joe {
      background:
        radial-gradient(ellipse 100% 70% at 50% -5%, #1a3a44 0%, transparent 65%),
        radial-gradient(ellipse 80% 40% at 80% 100%, #3a3428 0%, transparent 55%),
        var(--bg);
    }
    body.joe .brand { color: #5ecfb8; }
    body.joe .tag {
      color: #9ecfc4;
      font-style: italic;
      letter-spacing: 0.01em;
    }
    body.joe .mark {
      transform: rotate(-8deg) scale(1.06);
    }
    body.joe .btn { background: #2eccb0; color: #102018; }
    body.joe .btn:hover { background: #4ad9c0; }
    body.joe .brand-bar.egg:hover {
      background: rgba(46, 204, 176, 0.12);
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
      margin-top: 20px;
      display: flex;
      flex-wrap: wrap;
      gap: 12px 14px;
      justify-content: center;
      align-items: center;
    }
    .btn {
      display: inline-block;
      padding: 12px 20px;
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
    .btn-ghost {
      background: transparent;
      color: var(--muted);
      border: 1px solid rgba(140, 140, 140, 0.35);
      font-weight: 500;
    }
    .btn-ghost:hover {
      background: rgba(255, 255, 255, 0.06);
      color: var(--text);
    }
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
    .center .actions { margin-top: 22px; }
`;
}

function shareExpiryLine(uploaded: Date | undefined): string {
  if (!uploaded || Number.isNaN(uploaded.getTime())) {
    return "Shared clip · Expires in about 7 days";
  }
  const expires = new Date(uploaded.getTime() + 7 * 24 * 60 * 60 * 1000);
  const date = expires.toLocaleDateString("en-US", {
    timeZone: "UTC",
    month: "short",
    day: "numeric",
    year: "numeric",
  });
  return `Shared clip · Expires ${date}`;
}

function playerHtml(
  id: string,
  baseUrl: string,
  hasThumb: boolean,
  expiryLine: string,
): string {
  const pageUrl = `${baseUrl}/c/${id}`;
  const rawAbs = `${pageUrl}?raw=1`;
  const rawRel = `/c/${id}?raw=1`;
  const thumbAbs = `${baseUrl}/c/${id}/thumb.jpg`;
  const thumbRel = `/c/${id}/thumb.jpg`;
  const posterAttr = hasThumb ? ` poster="${thumbRel}"` : "";
  const ogImage = hasThumb
    ? `
  <meta property="og:image" content="${thumbAbs}" />
  <meta property="og:image:type" content="image/jpeg" />
  <meta property="og:image:width" content="1280" />
  <meta property="og:image:height" content="720" />`
    : "";
  const expiryEsc = expiryLine
    .replace(/&/g, "&amp;")
    .replace(/"/g, "&quot;")
    .replace(/</g, "&lt;");
  const expiryJs = JSON.stringify(expiryLine);

  return `<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="utf-8" />
  <meta name="viewport" content="width=device-width, initial-scale=1" />
  <title>ReplayForge — Shared clip</title>
  <meta property="og:type" content="video.other" />
  <meta property="og:site_name" content="ReplayForge" />
  <meta property="og:title" content="ReplayForge — Shared clip" />
  <meta property="og:description" content="${expiryEsc}" />
  <meta property="og:url" content="${pageUrl}" />
  <meta property="og:video" content="${rawAbs}" />
  <meta property="og:video:secure_url" content="${rawAbs}" />
  <meta property="og:video:type" content="video/mp4" />
  <meta property="og:video:width" content="1280" />
  <meta property="og:video:height" content="720" />${ogImage}
  <link rel="preconnect" href="https://fonts.googleapis.com" />
  <link rel="preconnect" href="https://fonts.gstatic.com" crossorigin />
  <link href="https://fonts.googleapis.com/css2?family=DM+Sans:opsz,wght@9..40,500;9..40,600;9..40,700&display=swap" rel="stylesheet" />
  <style>${sharedPageCss()}</style>
</head>
<body>
  <div class="brand-bar egg" id="brand-egg" title="">
    ${brandMarkSvg()}
    <h1 class="brand">ReplayForge</h1>
  </div>
  <p class="tag" id="tagline">${expiryEsc}</p>
  <div class="stage">
    <video controls playsinline preload="metadata"${posterAttr} src="${rawRel}"></video>
    <div class="actions">
      <a class="btn" id="dl-btn" href="${rawRel}" download>Download MP4</a>
      <a class="btn btn-ghost" href="${GITHUB_RELEASES}">Get ReplayForge</a>
    </div>
  </div>
  <script>
    (function () {
      var clicks = 0;
      var joe = false;
      var brand = document.getElementById("brand-egg");
      var tag = document.getElementById("tagline");
      var btn = document.getElementById("dl-btn");
      var normalTag = ${expiryJs};
      var joeTag = "Chicken Joe says it's all good, brah";
      if (!brand || !tag || !btn) return;
      brand.addEventListener("click", function () {
        clicks += 1;
        if (clicks < 7) return;
        clicks = 0;
        joe = !joe;
        document.body.classList.toggle("joe", joe);
        tag.textContent = joe ? joeTag : normalTag;
        btn.textContent = joe ? "Gnarly MP4" : "Download MP4";
      });
    })();
  </script>
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
    <p>Clip not found or expired · links last about 7 days</p>
    <div class="actions">
      <a class="btn" href="${GITHUB_RELEASES}">Get ReplayForge</a>
    </div>
  </div>
</body>
</html>`;
}

async function presignPut(
  env: Env,
  key: string,
  contentType: string,
): Promise<string> {
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
      headers: { "Content-Type": contentType },
    }),
    { aws: { signQuery: true } },
  );
  return signed.url;
}

function objectKey(id: string): string {
  return `clips/${id}.mp4`;
}

function thumbKey(id: string): string {
  return `clips/${id}.jpg`;
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
      // Keep client-facing errors generic — do not leak secret names.
      throw new Error("share service misconfigured");
    }
  }
}

function requireHttpsPublicBase(env: Env): void {
  const base = (env.PUBLIC_BASE_URL || "").trim();
  if (!base.startsWith("https://")) {
    throw new Error("share service misconfigured");
  }
}

/** Soft per-IP rate limit via Cache API. */
async function allowRate(
  request: Request,
  bucket: "init" | "complete" | "get",
  limit: number,
): Promise<boolean> {
  const ip =
    request.headers.get("CF-Connecting-IP") ||
    request.headers.get("X-Forwarded-For")?.split(",")[0]?.trim() ||
    "unknown";
  const minute = Math.floor(Date.now() / 60_000);
  const cacheKey = new Request(
    `https://replayforge-share.rate/${bucket}/${ip}/${minute}`,
  );
  const cache = caches.default;
  const existing = await cache.match(cacheKey);
  const count = existing ? Number(await existing.text()) : 0;
  if (count >= limit) {
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
