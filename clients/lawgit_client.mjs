// lawgit_client.mjs - dependency-free client for `lawgit serve` (Node 18+ / browsers with --cors-origin).
//
//   import { LawGitClient } from "./lawgit_client.mjs";
//   const c = new LawGitClient("http://127.0.0.1:8787", { token: "..." });
//   const rule = await c.createRule({ name: "休暇規則", category: "人事", body: "# 休暇" });
//   await c.editRule(rule.uuid, (r) => ({ body: r.body + "\n- 追記" }));   // conflict-safe

export class LawGitError extends Error {
  constructor(message, status = 0) { super(message); this.name = "LawGitError"; this.status = status; }
}
export class NotFound extends LawGitError {}
export class Conflict extends LawGitError {}

const clean = (o) => Object.fromEntries(Object.entries(o).filter(([, v]) => v !== undefined && v !== null));

export class LawGitClient {
  constructor(baseUrl = "http://127.0.0.1:8787", { token = null, fetchImpl = globalThis.fetch } = {}) {
    this.base = baseUrl.replace(/\/+$/, "") + "/api/v1";
    this.token = token;
    this.fetch = fetchImpl;
  }

  async _request(method, path, { query, body, raw = false } = {}) {
    let url = this.base + path;
    const q = new URLSearchParams(clean(query || {})).toString();
    if (q) url += "?" + q;
    const headers = { Accept: "application/json" };
    if (this.token) headers.Authorization = `Bearer ${this.token}`;
    let payload;
    if (body !== undefined || ["POST", "PUT", "PATCH"].includes(method)) {
      payload = JSON.stringify(clean(body || {}));
      headers["Content-Type"] = "application/json";
    }
    let res;
    try { res = await this.fetch(url, { method, headers, body: payload }); }
    catch (e) { throw new LawGitError(`cannot reach ${url}: ${e.message}`); }
    let doc;
    try { doc = await res.json(); } catch { throw new LawGitError(`non-JSON response (HTTP ${res.status})`, res.status); }
    if (raw && res.status === 200) return doc;
    if (!doc || doc.ok !== true) {
      const Cls = res.status === 404 ? NotFound : res.status === 409 ? Conflict : LawGitError;
      throw new Cls((doc && doc.error) || "unknown error", res.status);
    }
    return doc.result;
  }

  rpc(method, params = {}) { return this._request("POST", "/rpc", { body: { method, params } }); }
  ping() { return this._request("GET", "/ping"); }
  validate() { return this._request("GET", "/validate"); }
  openapi() { return this._request("GET", "/openapi.json", { raw: true }); }
  async search(q, limit) { return (await this._request("GET", "/search", { query: { q, limit } })).hits; }
  changes({ since, limit } = {}) { return this._request("GET", "/changes", { query: { since, limit } }); }
  commit(message) { return this._request("POST", "/git/commit", { body: { message } }); }

  async laws() { return (await this._request("GET", "/laws")).laws; }
  law(uuid) { return this._request("GET", `/laws/${uuid}`); }
  createLaw({ name, type, lawNumber, eGovId, commitMessage }) {
    return this._request("POST", "/laws", { body: { name, type, law_number: lawNumber, e_gov_id: eGovId, commit_message: commitMessage } });
  }

  async rules({ category, query } = {}) { return (await this._request("GET", "/rules", { query: { category, query } })).rules; }
  rule(uuid) { return this._request("GET", `/rules/${uuid}`); }
  createRule({ name, category, body = "", commitMessage }) {
    return this._request("POST", "/rules", { body: { name, category, body, commit_message: commitMessage } });
  }
  updateRule(uuid, { name, category, body, ifVersion, commitMessage } = {}) {
    return this._request("PUT", `/rules/${uuid}`, { body: { name, category, body, if_version: ifVersion, commit_message: commitMessage } });
  }
  deleteRule(uuid, { ifVersion, commitMessage } = {}) {
    return this._request("DELETE", `/rules/${uuid}`, { query: { if_version: ifVersion, commit_message: commitMessage } });
  }
  async editRule(uuid, change, { retries = 5, commitMessage } = {}) {
    for (let attempt = 0; attempt < retries; attempt++) {
      const current = await this.rule(uuid);
      const fields = await change(current);
      if (!fields) return current;
      try { return await this.updateRule(uuid, { ...fields, ifVersion: current.version, commitMessage }); }
      catch (e) { if (!(e instanceof Conflict) || attempt === retries - 1) throw e; }
    }
    throw new Conflict("too many conflicting edits", 409);
  }

  async history(kind, uuid, limit) { return (await this._request("GET", `/${kind}/${uuid}/history`, { query: { limit } })).commits; }
  async revision(kind, uuid, rev) { return (await this._request("GET", `/${kind}/${uuid}/revisions/${rev}`)).xml; }

  /** Async generator yielding new commits (oldest first); starts from "now" unless fromBeginning. */
  async *watch({ since, intervalMs = 2000, signal, fromBeginning = false } = {}) {
    if (!since && !fromBeginning) since = (await this.changes({ limit: 1 })).head;
    while (!(signal && signal.aborted)) {
      const res = await this.changes({ since, limit: 500 });
      for (const c of [...res.commits].reverse()) yield c;
      if (res.head) since = res.head;
      await new Promise((r) => setTimeout(r, intervalMs));
    }
  }
}
