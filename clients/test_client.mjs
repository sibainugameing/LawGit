import test from "node:test";
import assert from "node:assert/strict";
import http from "node:http";
import { LawGitClient, NotFound, Conflict, LawGitError } from "./lawgit_client.mjs";

let version = 1;
const log = [];
const server = http.createServer((req, res) => {
  let data = "";
  req.on("data", (d) => (data += d));
  req.on("end", () => {
    const u = new URL(req.url, "http://x");
    const body = data ? JSON.parse(data) : null;
    log.push({ method: req.method, path: u.pathname, q: Object.fromEntries(u.searchParams), body, auth: req.headers.authorization });
    const send = (code, doc) => { res.writeHead(code, { "Content-Type": "application/json" }); res.end(JSON.stringify(doc)); };
    if (req.headers.authorization !== "Bearer tok") return send(401, { ok: false, error: "unauthorized" });
    if (u.pathname === "/api/v1/rules/missing") return send(404, { ok: false, error: "not found: rule" });
    if (u.pathname === "/api/v1/rules/u1" && req.method === "GET") return send(200, { ok: true, result: { uuid: "u1", body: "b", version: `v${version}` } });
    if (u.pathname === "/api/v1/rules/u1" && req.method === "PUT") {
      if (body.if_version !== `v${version}`) return send(409, { ok: false, error: "conflict: modified" });
      version++; return send(200, { ok: true, result: { uuid: "u1", body: body.body, version: `v${version}` } });
    }
    if (u.pathname === "/api/v1/search") return send(200, { ok: true, result: { hits: [{ q: u.searchParams.get("q") }] } });
    if (u.pathname === "/api/v1/changes") {
      if (u.searchParams.get("limit") === "1") return send(200, { ok: true, result: { head: "h1", commits: [] } });
      return send(200, { ok: true, result: { head: "h3", commits: [{ hash: "h3", subject: "second" }, { hash: "h2", subject: "first" }] } });
    }
    send(404, { ok: false, error: "not found: route" });
  });
});
await new Promise((r) => server.listen(0, "127.0.0.1", r));
const url = `http://127.0.0.1:${server.address().port}`;
test.after(() => server.close());

test("auth, query encoding, errors", async () => {
  const c = new LawGitClient(url, { token: "tok" });
  const hits = await c.search("有給 休暇", 5);
  assert.equal(hits[0].q, "有給 休暇");
  assert.deepEqual(log.at(-1).q, { q: "有給 休暇", limit: "5" });
  await assert.rejects(c.rule("missing"), NotFound);
  await assert.rejects(new LawGitClient(url, { token: "bad" }).search("x"), (e) => e instanceof LawGitError && e.status === 401);
});

test("editRule retries on conflict", async () => {
  version = 1;
  const c = new LawGitClient(url, { token: "tok" });
  let n = 0;
  const out = await c.editRule("u1", (r) => { if (++n === 1) version += 5; return { body: "new" }; });
  assert.equal(n, 2);
  assert.equal(out.body, "new");
  const puts = log.filter((e) => e.method === "PUT").slice(-2);
  assert.deepEqual(puts.map((p) => p.body.if_version), ["v1", "v6"]);
  await assert.rejects(c.editRule("u1", () => { version += 1; return { body: "x" }; }, { retries: 2 }), Conflict);
});

test("watch yields oldest first", async () => {
  const c = new LawGitClient(url, { token: "tok" });
  const ac = new AbortController();
  const got = [];
  for await (const commit of c.watch({ intervalMs: 1, signal: ac.signal })) { got.push(commit.subject); if (got.length === 2) ac.abort(); }
  assert.deepEqual(got, ["first", "second"]);
});
