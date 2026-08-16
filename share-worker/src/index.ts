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
        return await handleClipGet(clipMatch[1], env);
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

async function handleClipGet(id: string, env: Env): Promise<Response> {
  const obj = await env.CLIPS.get(objectKey(id));
  if (!obj) {
    return new Response("Clip not found or expired", { status: 404 });
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
