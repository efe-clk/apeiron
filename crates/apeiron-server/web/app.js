// The desktop and web interface (DESKTOP.md §2, WEB.md §3). It speaks only to
// the JSON API of the server (DESKTOP.md §3, WEB.md §4).

import { t, lang } from "./i18n.js";

document.documentElement.lang = lang;
document.title = t("title");

// The token comes once in the address fragment and stays in this tab only.
const fragment = new URLSearchParams(location.hash.slice(1));
if (fragment.get("token")) {
  sessionStorage.setItem("apeiron-token", fragment.get("token"));
  history.replaceState(null, "", "/");
}
const token = sessionStorage.getItem("apeiron-token") || "";
// A sign-in through the identity provider that did not work (IDENTITY.md §4).
const oidcError = fragment.get("oidc_error");
if (oidcError) history.replaceState(null, "", "/");
// Whether the sign-in screen offers the identity provider.
let oidc = false;
// "local" with a token, or "web" with a sign-in cookie.
let mode = "local";
// The signed-in user in web mode: {name, role} (USERS.md §4).
let user = null;
const may = {
  backUp: () => user?.role !== "reader",
  restore: () => user?.role !== "reader",
  verify: () => !user || user.role === "admin",
  readLog: () => user?.role === "admin",
  share: () => user !== null && user.role !== "reader",
  manageUsers: () => user?.role === "admin",
};

const main = document.getElementById("main");
const nav = document.getElementById("nav");

// ---- The JSON API ----

class ApiError extends Error {
  constructor(value) {
    super(value?.message || t("error"));
    this.kind = value?.kind || "internal";
    this.signIn = value?.sign_in === true;
  }
}

async function api(method, path, body) {
  const response = await fetch(path, {
    method,
    headers: {
      ...(token ? { Authorization: `Bearer ${token}` } : {}),
      ...(body === undefined ? {} : { "Content-Type": "application/json" }),
    },
    body: body === undefined ? undefined : JSON.stringify(body),
  });
  const value = await response.json().catch(() => null);
  if (!response.ok) {
    const err = new ApiError(value);
    // A session that ended sends the user back to the sign-in screen.
    if (err.signIn && path !== "/api/login") signIn();
    throw err;
  }
  return value;
}

const call = (operation, request = {}) => api("POST", `/api/call/${operation}`, request);

// Starts a long operation and reports its progress until it ends. The
// returned object can be cancelled; `done` holds the result.
function run(operation, request, onProgress) {
  let id = null;
  let cancelled = false;
  const done = (async () => {
    id = (await api("POST", `/api/start/${operation}`, request)).job;
    if (cancelled) await api("POST", `/api/jobs/${id}/cancel`).catch(() => {});
    for (;;) {
      const state = await api("GET", `/api/jobs/${id}`);
      onProgress(state.progress);
      if (state.done) {
        if (state.error) throw new ApiError(state.error);
        return state.result;
      }
      await new Promise((resolve) => setTimeout(resolve, 250));
    }
  })();
  return {
    done,
    cancel() {
      cancelled = true;
      if (id !== null) api("POST", `/api/jobs/${id}/cancel`).catch(() => {});
    },
  };
}

// ---- Small helpers ----

// An element with attributes (`on…` are listeners) and children.
function h(tag, attrs = {}, ...children) {
  const el = document.createElement(tag);
  for (const [key, value] of Object.entries(attrs || {})) {
    if (value === undefined || value === null || value === false) continue;
    if (key.startsWith("on")) el.addEventListener(key.slice(2), value);
    else if (key === "class") el.className = value;
    else if (value === true) el.setAttribute(key, "");
    else el.setAttribute(key, value);
  }
  for (const child of children.flat()) {
    if (child === null || child === undefined || child === false) continue;
    el.append(child instanceof Node ? child : String(child));
  }
  return el;
}

function show(...nodes) {
  main.replaceChildren(...nodes.filter((n) => n !== null && n !== undefined && n !== false));
  main.focus();
  document.documentElement.scrollTop = 0;
}

function bytes(n) {
  const units = ["B", "KB", "MB", "GB", "TB"];
  let value = n;
  let unit = 0;
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024;
    unit += 1;
  }
  const digits = unit === 0 || value >= 100 ? 0 : 1;
  return `${value.toLocaleString(lang, { maximumFractionDigits: digits })} ${units[unit]}`;
}

function when(time) {
  if (!time) return "";
  return new Date(time.seconds * 1000).toLocaleString(lang, { dateStyle: "medium", timeStyle: "short" });
}

function message(kind, text) {
  return h("div", { class: `message ${kind}`, role: kind === "bad" ? "alert" : "status" }, text);
}

function errorMessage(err) {
  if (err.kind === "cancelled") return message("", t("cancelled"));
  return message("bad", `${t("error")}: ${err.message}`);
}

function field(labelText, input, hint) {
  const id = `f${Math.random().toString(36).slice(2)}`;
  input.id = id;
  return [h("label", { for: id }, labelText, hint ? h("span", { class: "hint" }, ` — ${hint}`) : null), input];
}

// A path input with a button that opens the folder picker.
function folderInput(value = "") {
  const input = h("input", { type: "text", value, spellcheck: "false", autocomplete: "off" });
  const choose = h("button", { type: "button", onclick: async () => {
    const path = await pickFolder(input.value);
    if (path !== null) {
      input.value = path;
      input.dispatchEvent(new Event("input"));
    }
  } }, t("choose"));
  return { input, row: h("div", { class: "row" }, input, choose) };
}

function separator(path) {
  return path.includes("\\") && !path.includes("/") ? "\\" : "/";
}

function join(parent, name) {
  const sep = separator(parent);
  return parent.endsWith(sep) ? parent + name : parent + sep + name;
}

function baseName(path) {
  const parts = path.split(/[\\/]+/).filter(Boolean);
  return parts[parts.length - 1] || "";
}

// A ref name from a folder name: lowercase letters, digits, - and _ (FORMAT.md §9.3).
function refFor(folder) {
  const name = baseName(folder)
    .toLowerCase()
    .normalize("NFKD")
    .replace(/[̀-ͯ]/g, "")
    .replace(/ı/g, "i")
    .replace(/[^a-z0-9_-]+/g, "-")
    .replace(/^-+|-+$/g, "");
  return `backup/${name || "folder"}`;
}

// ---- The folder picker ----

const picker = document.getElementById("picker");
const pickerPath = document.getElementById("picker-path");
const pickerList = document.getElementById("picker-list");
let pickerParent = null;
let pickerResolve = null;

document.getElementById("picker-title").textContent = t("choose_folder");
document.getElementById("picker-up").textContent = t("up");
document.getElementById("picker-cancel").textContent = t("cancel");
document.getElementById("picker-ok").textContent = t("choose_this");
pickerPath.setAttribute("aria-label", t("path"));

async function listFolders(path) {
  try {
    const listing = await api("GET", `/api/folders?path=${encodeURIComponent(path || "")}`);
    pickerPath.value = listing.path;
    pickerParent = listing.parent;
    document.getElementById("picker-up").disabled = !listing.parent;
    pickerList.replaceChildren(
      ...listing.folders.map((name) =>
        h("li", {}, h("button", { type: "button", onclick: () => listFolders(join(listing.path, name)) }, `📁 ${name}`)),
      ),
    );
  } catch (err) {
    pickerList.replaceChildren(h("li", {}, errorMessage(err)));
  }
}

function pickFolder(start) {
  listFolders(start);
  picker.showModal();
  return new Promise((resolve) => {
    pickerResolve = resolve;
  });
}

function closePicker(value) {
  picker.close();
  if (pickerResolve) pickerResolve(value);
  pickerResolve = null;
}

document.getElementById("picker-up").addEventListener("click", () => pickerParent && listFolders(pickerParent));
document.getElementById("picker-cancel").addEventListener("click", () => closePicker(null));
document.getElementById("picker-ok").addEventListener("click", () => closePicker(pickerPath.value));
document.getElementById("picker-form").addEventListener("submit", (e) => {
  e.preventDefault();
  listFolders(pickerPath.value);
});
picker.addEventListener("cancel", () => closePicker(null));

// ---- Progress of a job ----

// Runs a long operation, showing its progress in `box` with a cancel
// button; resolves to its result, or shows the error and resolves to null.
async function runWithProgress(box, operation, request) {
  const phase = h("p", {}, t("phase_starting"));
  const bar = h("progress", {});
  const counts = h("p", { class: "muted" });
  const update = (p) => {
    if (!p) return;
    phase.textContent = t(`phase_${p.phase}`);
    if (p.bytes_total) {
      bar.max = p.bytes_total;
      bar.value = p.bytes_done;
    } else if (p.items_total) {
      bar.max = p.items_total;
      bar.value = p.items_done;
    }
    const items = p.items_total
      ? t("progress_items", { done: p.items_done, total: p.items_total })
      : t("progress_items_open", { done: p.items_done });
    counts.textContent = `${items} · ${bytes(p.bytes_done)}${p.current ? ` · ${p.current}` : ""}`;
  };
  const job = run(operation, request, update);
  const cancel = h("button", { type: "button", onclick: () => job.cancel() }, t("cancel"));
  box.replaceChildren(h("div", { class: "card", role: "status" }, phase, bar, counts, h("div", { class: "actions" }, cancel)));
  try {
    const result = await job.done;
    box.replaceChildren();
    return result;
  } catch (err) {
    box.replaceChildren(errorMessage(err));
    return null;
  }
}

// ---- Navigation ----

let repoPath = "";

function showNav(current) {
  const item = (key, view) =>
    h("button", { type: "button", "aria-current": current === key ? "page" : false, onclick: view }, t(`nav_${key}`));
  nav.hidden = false;
  nav.replaceChildren(...[
    item("overview", overview),
    may.backUp() ? item("backup", () => backup()) : null,
    may.verify() ? item("verify", verify) : null,
    may.verify() ? item("report", report) : null,
    may.verify() ? item("retention", retention) : null,
    mode === "local" ? item("sync", () => sync()) : null,
    mode === "local" ? item("plugins", plugins) : null,
    may.share() ? item("shares", shares) : null,
    may.readLog() ? item("activity", activity) : null,
    may.manageUsers() ? item("users", users) : null,
    mode === "web" ? item("sign_out", signOut) : item("close", () => start()),
    h("span", { class: "path" }, user ? `${user.name} · ${repoPath}` : repoPath),
  ].filter(Boolean));
}

// ---- Start ----

async function start() {
  nav.hidden = true;
  show(h("p", {}, t("loading")));
  const state = await api("GET", "/api/state").catch(() => ({ open: null, last: null }));
  const last = state.open?.path || state.last || "";

  const open = folderInput(last);
  const password = h("input", { type: "password", autocomplete: "current-password" });
  const passwordField = h("div", { hidden: true }, field(t("password"), password));
  const openStatus = h("div");
  const openForm = h("form", { onsubmit: async (e) => {
    e.preventDefault();
    openStatus.replaceChildren();
    try {
      const info = await api("POST", "/api/open", { path: open.input.value, password: password.value || null });
      opened(info.path);
    } catch (err) {
      if (err.kind === "key") {
        passwordField.hidden = false;
        password.focus();
        openStatus.replaceChildren(message("", t("password_needed")));
      } else {
        openStatus.replaceChildren(errorMessage(err));
      }
    }
  } },
    h("h2", {}, t("open_heading")),
    last ? h("p", { class: "muted" }, `${t("last_used")}: ${last}`) : null,
    h("label", { for: "open-path" }, t("path")),
    open.row,
    passwordField,
    openStatus,
    h("div", { class: "actions" }, h("button", { type: "submit", class: "primary" }, t("open_button"))),
  );
  open.input.id = "open-path";

  const made = folderInput("");
  const newPassword = h("input", { type: "password", autocomplete: "new-password" });
  const again = h("input", { type: "password", autocomplete: "new-password" });
  const newStatus = h("div");
  const newForm = h("form", { onsubmit: async (e) => {
    e.preventDefault();
    newStatus.replaceChildren();
    if (newPassword.value !== again.value) {
      newStatus.replaceChildren(message("bad", t("passwords_differ")));
      return;
    }
    try {
      const answer = await api("POST", "/api/init", { path: made.input.value, password: newPassword.value || null });
      if (answer.recovery_key) recovery(answer.recovery_key, answer.info.path);
      else opened(answer.info.path);
    } catch (err) {
      newStatus.replaceChildren(errorMessage(err));
    }
  } },
    h("h2", {}, t("new_heading")),
    h("p", { class: "muted" }, t("new_intro")),
    h("label", { for: "new-path" }, t("path")),
    made.row,
    field(t("password_optional"), newPassword),
    field(t("password_again"), again),
    newStatus,
    h("div", { class: "actions" }, h("button", { type: "submit" }, t("new_button"))),
  );
  made.input.id = "new-path";

  show(
    h("h1", {}, t("start_heading")),
    h("p", { class: "muted" }, t("start_intro")),
    h("div", { class: "card" }, openForm),
    h("div", { class: "card" }, newForm),
  );
}

function recovery(key, path) {
  show(
    h("h1", {}, t("recovery_heading")),
    h("div", { class: "card" },
      h("p", {}, t("recovery_text")),
      message("key", key),
      h("div", { class: "actions" }, h("button", { type: "button", class: "primary", onclick: () => opened(path) }, t("recovery_done"))),
    ),
  );
}

function opened(path) {
  repoPath = path;
  overview();
}

// ---- Overview ----

async function overview() {
  showNav("overview");
  show(h("p", {}, t("loading")));
  try {
    // `folder/…` refs hold the state of synced folders (SYNC.md §3).
    const refs = (await call("refs")).filter((r) => !r.name.startsWith("folder/"));
    if (refs.length === 0) {
      show(
        h("h1", {}, t("overview_heading")),
        h("div", { class: "card" },
          h("p", {}, t("overview_empty")),
          may.backUp() ? h("div", { class: "actions" }, h("button", { type: "button", class: "primary", onclick: () => backup() }, t("nav_backup"))) : null,
        ),
      );
      return;
    }
    const rows = await Promise.all(refs.map(async (r) => {
      const page = await call("log", { start: r.name, limit: 1 });
      const newest = page.snapshots[0];
      const source = newest?.meta?.find(([k]) => k === "source")?.[1];
      return h("tr", {},
        h("td", {}, h("strong", {}, r.name.replace(/^backup\//, "")), source ? h("div", { class: "muted" }, source) : null),
        h("td", {}, when(newest?.time)),
        h("td", {}, h("div", { class: "actions" },
          h("button", { type: "button", onclick: () => historyOf(r.name) }, t("history")),
          source && may.backUp() ? h("button", { type: "button", onclick: () => backup(source, r.name) }, t("back_up_again")) : null,
        )),
      );
    }));
    show(
      h("h1", {}, t("overview_heading")),
      h("div", { class: "card" },
        h("table", {},
          h("thead", {}, h("tr", {}, h("th", {}, t("folder_of")), h("th", {}, t("last_backup")), h("th", {}, ""))),
          h("tbody", {}, rows),
        ),
      ),
    );
  } catch (err) {
    show(errorMessage(err));
  }
}

// ---- Back up ----

function backup(source = "", ref = "") {
  showNav("backup");
  const folder = folderInput(source);
  const name = h("input", { type: "text", value: ref || (source ? refFor(source) : ""), spellcheck: "false" });
  let nameEdited = Boolean(ref);
  name.addEventListener("input", () => { nameEdited = true; });
  folder.input.addEventListener("input", () => {
    if (!nameEdited) name.value = refFor(folder.input.value);
  });
  const note = h("input", { type: "text" });
  const status = h("div");
  const button = h("button", { type: "submit", class: "primary" }, t("backup_button"));
  const form = h("form", { onsubmit: async (e) => {
    e.preventDefault();
    button.disabled = true;
    const path = folder.input.value;
    const result = await runWithProgress(status, "backup", {
      source: path,
      ref: name.value,
      message: note.value || null,
      meta: [["source", path]],
    });
    button.disabled = false;
    if (!result) return;
    const done = [message("good", t("backup_done", { files: result.files, bytes: bytes(result.bytes), new_bytes: bytes(result.new_bytes) }))];
    if (result.warnings.length) {
      done.push(message("bad", [t("warnings"), ...result.warnings.map((w) => h("div", {}, `${w.path}: ${w.message}`))]));
    }
    done.push(h("div", { class: "actions" }, h("button", { type: "button", onclick: () => historyOf(name.value) }, t("history"))));
    status.replaceChildren(...done);
  } },
    h("label", { for: "backup-path" }, t("path")),
    folder.row,
    field(t("backup_name"), name, t("backup_name_hint")),
    field(t("note"), note),
    h("div", { class: "actions" }, button),
  );
  folder.input.id = "backup-path";
  show(h("h1", {}, t("backup_heading")), h("p", { class: "muted" }, t("backup_intro")), h("div", { class: "card" }, form), status);
}

// ---- History ----

async function historyOf(ref, from = ref) {
  showNav("overview");
  show(h("p", {}, t("loading")));
  try {
    const page = await call("log", { start: from, limit: 50 });
    const rows = page.snapshots.map((s) =>
      h("tr", {},
        h("td", {}, when(s.time), s.message ? h("div", { class: "muted" }, s.message) : null),
        h("td", {}, h("div", { class: "actions" },
          h("button", { type: "button", onclick: () => browse(s, "") }, t("browse")),
          h("button", { type: "button", onclick: () => compare(s) }, t("compare")),
          may.restore() ? h("button", { type: "button", onclick: () => restore(s, "") }, t("restore")) : null,
        )),
      ),
    );
    show(
      h("h1", {}, t("history_heading", { name: ref.replace(/^backup\//, "") })),
      h("div", { class: "card" }, h("table", {}, h("tbody", {}, rows))),
      page.next ? h("button", { type: "button", onclick: () => historyOf(ref, page.next) }, "…") : null,
    );
  } catch (err) {
    show(errorMessage(err));
  }
}

// ---- Browse ----

async function browse(snapshot, path) {
  showNav("overview");
  show(h("p", {}, t("loading")));
  try {
    const entries = await call("list", { snapshot: snapshot.id, path });
    const parts = path.split("/").filter(Boolean);
    const crumbs = h("div", { class: "crumbs" },
      h("button", { type: "button", class: "link", onclick: () => browse(snapshot, "") }, "/"),
      parts.map((part, i) => [
        " / ",
        h("button", { type: "button", class: "link", onclick: () => browse(snapshot, parts.slice(0, i + 1).join("/")) }, part),
      ]),
    );
    const rows = entries.map((e) => {
      const full = path ? `${path}/${e.name}` : e.name;
      const name = e.kind === "dir"
        ? h("button", { type: "button", class: "link", onclick: () => browse(snapshot, full) }, `📁 ${e.name}`)
        : `${e.kind === "symlink" ? "🔗" : "📄"} ${e.name}`;
      return h("tr", {},
        h("td", {}, name),
        h("td", { class: "num" }, e.size === null ? "" : bytes(e.size)),
        h("td", {}, when(e.mtime)),
        h("td", {}, h("div", { class: "actions" },
          may.restore() ? h("button", { type: "button", onclick: () => restore(snapshot, full) }, t("restore_this")) : null,
          may.share() && e.kind !== "symlink" ? h("button", { type: "button", onclick: () => share(snapshot, full) }, t("share")) : null,
        )),
      );
    });
    show(
      h("h1", {}, t("browse_heading", { time: when(snapshot.time) })),
      h("div", { class: "card" }, crumbs, rows.length ? h("table", {}, h("tbody", {}, rows)) : h("p", { class: "muted" }, t("empty_folder"))),
      may.share() ? h("div", { class: "actions" }, h("button", { type: "button", onclick: () => share(snapshot, path) }, t("share_folder"))) : null,
    );
  } catch (err) {
    show(errorMessage(err));
  }
}

// ---- Restore ----

function restore(snapshot, path) {
  showNav("overview");
  const time = when(snapshot.time);
  const what = path ? t("restore_what_path", { path, time }) : t("restore_what_all", { time });
  const target = folderInput("");
  const d = new Date(snapshot.time.seconds * 1000);
  const date = `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}-${String(d.getDate()).padStart(2, "0")}`;
  const newName = h("input", { type: "text", value: `${baseName(path) || "apeiron"} ${date}`, spellcheck: "false" });
  const status = h("div");
  const button = h("button", { type: "submit", class: "primary" }, t("restore_button"));
  const form = h("form", { onsubmit: async (e) => {
    e.preventDefault();
    button.disabled = true;
    const into = newName.value.trim() ? join(target.input.value, newName.value.trim()) : target.input.value;
    const result = await runWithProgress(status, "restore", { snapshot: snapshot.id, target: into, path });
    button.disabled = false;
    if (result) {
      status.replaceChildren(message("good", t("restore_done", { files: result.files, bytes: bytes(result.bytes) })), h("p", { class: "muted" }, into));
    }
  } },
    h("p", {}, h("strong", {}, what)),
    h("p", { class: "muted" }, t("restore_intro")),
    h("label", { for: "restore-path" }, t("restore_target")),
    target.row,
    field(t("new_folder_name"), newName),
    h("div", { class: "actions" }, button),
  );
  target.input.id = "restore-path";
  show(h("h1", {}, t("restore_heading")), h("div", { class: "card" }, form), status);
}

// ---- Compare ----

function diffLines(text) {
  return h("pre", { class: "diff" }, text.split("\n").filter((l, i, all) => l || i < all.length - 1).map((line) => {
    const kind = line.startsWith("@@") ? "hunk" : line.startsWith("+") ? "add" : line.startsWith("-") ? "del" : "";
    return h("span", { class: kind }, line);
  }));
}

async function compare(snapshot) {
  showNav("overview");
  show(h("p", {}, t("loading")));
  const heading = h("h1", {}, t("compare_heading", { time: when(snapshot.time) }));
  try {
    if (!snapshot.parents.length) {
      show(heading, h("div", { class: "card" }, h("p", {}, t("compare_first"))));
      return;
    }
    const changes = await call("diff_text", { from: snapshot.parents[0], to: snapshot.id });
    const items = changes.map((c) =>
      h("div", { class: "change" },
        h("h3", {}, h("span", { class: "badge" }, t(c.kind)), c.path),
        c.text ? diffLines(c.text) : null,
      ),
    );
    show(heading, h("div", { class: "card" }, items.length ? items : h("p", {}, t("compare_none"))));
  } catch (err) {
    show(heading, errorMessage(err));
  }
}

// ---- Verify ----

function verify() {
  showNav("verify");
  const status = h("div");
  const button = h("button", { type: "button", class: "primary", onclick: async () => {
    button.disabled = true;
    const result = await runWithProgress(status, "verify", {});
    button.disabled = false;
    if (!result) return;
    const damaged = result.damaged_files.length + result.damaged_objects.length;
    status.replaceChildren(damaged
      ? message("bad", t("verify_bad", { count: damaged }))
      : message("good", t("verify_ok", { objects: result.objects_checked.toLocaleString(lang) })));
  } }, t("verify_button"));
  show(h("h1", {}, t("verify_heading")), h("div", { class: "card" }, h("p", { class: "muted" }, t("verify_intro")), h("div", { class: "actions" }, button)), status);
}

// ---- Compliance report (REPORTS.md) ----

function report() {
  showNav("report");
  const verifyBox = h("input", { type: "checkbox" });
  const remotesBox = h("input", { type: "checkbox" });
  const status = h("div");
  const output = h("div");
  const button = h("button", { type: "button", class: "primary", onclick: async () => {
    button.disabled = true;
    output.replaceChildren();
    const r = await runWithProgress(status, "report", { verify: verifyBox.checked, remotes: remotesBox.checked });
    button.disabled = false;
    if (r) output.replaceChildren(...reportView(r));
  } }, t("report_button"));
  show(
    h("h1", {}, t("report_heading")),
    h("div", { class: "card" },
      h("p", { class: "muted" }, t("report_intro")),
      h("label", {}, verifyBox, " ", t("report_verify")),
      h("label", {}, remotesBox, " ", t("report_remotes")),
      h("div", { class: "actions" }, button)),
    status,
    output,
  );
}

// A finding in the page's language; `text` is English (REPORTS.md §4).
function findingText(f) {
  const key = f.code === "copy" ? `finding_copy_${f.state}` : `finding_${f.code}`;
  const text = t(key, f);
  return text === key ? f.text : text;
}

function reportView(r) {
  const table = (heads, rows) => h("div", { class: "card scroll" }, h("table", {},
    h("thead", {}, h("tr", {}, ...heads.map((k) => h("th", {}, t(k))))),
    h("tbody", {}, ...rows)));
  const findings = r.findings.length
    ? r.findings.map((f) => message(f.level === "problem" ? "bad" : "", findingText(f)))
    : [message("good", t("report_no_findings"))];
  const facts = [
    ["report_encrypted", r.repository.encrypted ? t("yes") : t("no")],
    ...(r.repository.encrypted ? [["report_passwords", r.repository.passwords], ["report_recovery_keys", r.repository.recovery_keys]] : []),
    ["report_verified", r.verify
      ? t("report_verified_result", { objects: r.verify.objects_checked.toLocaleString(lang), damaged: r.verify.damaged_files.length + r.verify.damaged_objects.length })
      : t("report_not_verified")],
  ];
  const refs = r.refs.map((f) => h("tr", {},
    h("td", {}, f.name, f.stale ? h("span", { class: "bad" }, ` (${t("report_stale")})`) : null),
    h("td", { class: "muted" }, f.snapshot.slice(0, 12)),
    h("td", {}, when(f.time)),
    h("td", { class: "num" }, f.snapshots),
    h("td", {}, f.retention_days ? t("report_days", { days: f.retention_days }) : "—"),
    h("td", {}, f.copies?.length ? f.copies.map((c) => `${c.remote}: ${t(`copy_${c.state}`)}`).join(", ") : "—"),
  ));
  return [
    h("h2", {}, t("report_findings")),
    ...findings,
    h("h2", {}, t("report_repository")),
    table(["report_item", "report_value"], facts.map(([k, v]) => h("tr", {}, h("td", {}, t(k)), h("td", {}, String(v))))),
    h("h2", {}, t("report_backups")),
    table(["report_ref", "report_snapshot", "report_newest", "report_snapshots", "report_kept", "report_copies"], refs),
    h("p", { class: "muted" }, t("report_proof")),
  ];
}

// ---- Remotes and sync (DESKTOP.md §3, SYNC.md) ----

async function sync(folderPath = "") {
  showNav("sync");
  show(h("p", {}, t("loading")));
  let remotes;
  try {
    remotes = await call("remotes");
  } catch (err) {
    show(h("h1", {}, t("sync_heading")), errorMessage(err));
    return;
  }
  const remoteRows = remotes.map((r) => h("tr", {},
    h("td", { class: "nowrap" }, r.name),
    h("td", {}, r.url),
    h("td", {}, h("button", { type: "button", onclick: async (e) => {
      if (!confirm(t("remote_remove_ask", { name: r.name }))) return;
      e.target.disabled = true;
      try {
        await call("remove_remote", { name: r.name });
        sync(folder.input.value);
      } catch (err) {
        main.append(errorMessage(err));
      }
    } }, t("remove")))));

  const name = h("input", { type: "text", autocomplete: "off", autocapitalize: "none", spellcheck: "false", required: true });
  const url = h("input", { type: "text", autocomplete: "off", spellcheck: "false", required: true });
  const remoteStatus = h("div");
  const addForm = h("form", { onsubmit: async (e) => {
    e.preventDefault();
    remoteStatus.replaceChildren();
    try {
      await call("add_remote", { name: name.value.trim(), url: url.value.trim() });
      sync(folder.input.value);
    } catch (err) {
      remoteStatus.replaceChildren(errorMessage(err));
    }
  } },
    ...field(t("remote_name"), name, t("remote_name_hint")),
    ...field(t("remote_url"), url, t("remote_url_hint")),
    h("div", { class: "actions" }, h("button", { type: "submit" }, t("remote_add"))),
  );

  const folder = folderInput(folderPath);
  folder.input.id = "sync-path";
  const remote = h("select", {}, ...remotes.map((r) => h("option", { value: r.name }, r.name)));
  const way = h("select", {}, ...["two-way", "send", "receive"].map((m) => h("option", { value: m }, t(`sync_mode_${m}`))));
  const dryRun = h("input", { type: "checkbox" });
  const status = h("div");
  const button = h("button", { type: "submit", class: "primary", disabled: remotes.length === 0 }, t("sync_button"));
  const syncForm = h("form", { onsubmit: async (e) => {
    e.preventDefault();
    button.disabled = true;
    const result = await runWithProgress(status, "sync", {
      folder: folder.input.value, remote: remote.value, mode: way.value, dry_run: dryRun.checked,
    });
    button.disabled = false;
    if (result) status.replaceChildren(...syncView(result, dryRun.checked), conflictList(folder.input.value));
  } },
    h("label", { for: "sync-path" }, t("sync_folder")),
    folder.row,
    ...field(t("sync_remote"), remote),
    ...field(t("sync_mode"), way),
    h("label", {}, dryRun, " ", t("sync_dry_run")),
    h("div", { class: "actions" }, button,
      h("button", { type: "button", onclick: () => status.replaceChildren(conflictList(folder.input.value)) }, t("conflicts_find"))),
  );

  show(
    h("h1", {}, t("sync_heading")),
    h("p", { class: "muted" }, t("sync_intro")),
    remotes.length ? h("div", { class: "card" }, syncForm) : message("", t("sync_no_remote")),
    status,
    h("h2", {}, t("remotes_heading")),
    remotes.length
      ? h("div", { class: "card scroll" }, h("table", {},
        h("thead", {}, h("tr", {}, ...["remote_name", "remote_url", ""].map((k) => h("th", {}, k ? t(k) : "")))),
        h("tbody", {}, ...remoteRows)))
      : null,
    h("div", { class: "card" }, addForm),
    remoteStatus,
  );
}

// The conflict copies in `folder`, each with buttons to keep one version.
function conflictList(folder) {
  const box = h("div", {}, h("p", {}, t("loading")));
  const load = async () => {
    try {
      const found = await call("conflicts", { folder });
      const resolve = (c, keep) => async (e) => {
        e.target.disabled = true;
        try {
          await call("resolve_conflict", { folder, copy: c.copy, keep });
          load();
        } catch (err) {
          box.append(errorMessage(err));
        }
      };
      box.replaceChildren(
        h("h2", {}, t("conflicts_heading")),
        found.length
          ? h("div", { class: "card scroll" }, h("p", { class: "muted" }, t("conflicts_intro")), h("table", {},
            h("thead", {}, h("tr", {}, ...["conflicts_file", "conflicts_copy", ""].map((k) => h("th", {}, k ? t(k) : "")))),
            h("tbody", {}, ...found.map((c) => h("tr", {},
              h("td", {}, c.path, c.exists ? null : h("span", { class: "muted" }, ` (${t("conflicts_missing")})`)),
              h("td", {}, c.copy),
              h("td", { class: "nowrap" },
                h("button", { type: "button", onclick: resolve(c, "this") }, t(c.exists ? "conflicts_keep_this" : "conflicts_keep_none")),
                h("button", { type: "button", onclick: resolve(c, "copy") }, t("conflicts_keep_copy"))))))))
          : message("good", t("conflicts_none")),
      );
    } catch (err) {
      box.replaceChildren(errorMessage(err));
    }
  };
  load();
  return box;
}

function syncView(r, dryRun) {
  const out = [message("good", t(dryRun ? "sync_would" : `sync_${r.action}`, { changes: r.changes.length }))];
  if (r.changes.length) {
    out.push(h("div", { class: "card scroll" }, h("table", {}, h("tbody", {},
      ...r.changes.map((c) => h("tr", {}, h("td", { class: "nowrap" }, t(c.kind)), h("td", {}, c.path)))))));
  }
  if (r.conflicts.length) {
    out.push(message("bad", [t("sync_conflicts"), ...r.conflicts.map((c) =>
      h("div", {}, t(`conflict_${c.kind}`, { path: c.path, copy: c.copy || "" })))]));
  }
  if (r.skipped.length) out.push(message("", [t("sync_skipped"), ...r.skipped.map((p) => h("div", {}, p))]));
  if (r.warnings.length) {
    out.push(message("bad", [t("warnings"), ...r.warnings.map((w) => h("div", {}, `${w.path}: ${w.message}`))]));
  }
  return out;
}

// ---- Plugins (PLUGIN.md §1) ----

async function plugins() {
  showNav("plugins");
  show(h("p", {}, t("loading")));
  let list;
  try {
    list = await call("plugins");
  } catch (err) {
    show(h("h1", {}, t("plugins_heading")), errorMessage(err));
    return;
  }
  const file = h("input", { type: "text", autocomplete: "off", spellcheck: "false", required: true });
  const status = h("div");
  const form = h("form", { onsubmit: async (e) => {
    e.preventDefault();
    status.replaceChildren();
    try {
      const added = await call("plugin_add", { file: file.value.trim() });
      await plugins();
      main.append(message("good", t(added.replaced ? "plugins_replaced" : "plugins_added", { name: added.info.name })));
    } catch (err) {
      status.replaceChildren(errorMessage(err));
    }
  } },
    ...field(t("plugins_file"), file, t("plugins_file_hint")),
    h("div", { class: "actions" }, h("button", { type: "submit", class: "primary" }, t("plugins_add"))),
  );
  const rows = list.map((p) => h("tr", {},
    h("td", { class: "nowrap" }, p.name, h("div", { class: "muted" }, p.version)),
    h("td", {}, p.description),
    h("td", {}, p.patterns.join(", ")),
    h("td", { class: "nowrap" }, h("button", { type: "button", onclick: async (e) => {
      if (!confirm(t("plugins_remove_ask", { name: p.name }))) return;
      e.target.disabled = true;
      try {
        await call("plugin_remove", { name: p.name });
        plugins();
      } catch (err) {
        main.append(errorMessage(err));
      }
    } }, t("remove")))));
  show(
    h("h1", {}, t("plugins_heading")),
    h("p", { class: "muted" }, t("plugins_intro")),
    list.length
      ? h("div", { class: "card scroll" }, h("table", {},
        h("thead", {}, h("tr", {}, ...["plugins_name", "plugins_description", "plugins_patterns", ""].map((k) => h("th", {}, k ? t(k) : "")))),
        h("tbody", {}, ...rows)))
      : h("p", {}, t("plugins_none")),
    h("h2", {}, t("plugins_add")),
    h("div", { class: "card" }, form),
    status,
  );
}

// ---- Retention rules (ARCHIVE.md §2) ----

async function retention() {
  showNav("retention");
  show(h("p", {}, t("loading")));
  let rules;
  try {
    rules = await call("retention");
  } catch (err) {
    show(h("h1", {}, t("retention_heading")), errorMessage(err));
    return;
  }
  const refs = h("input", { type: "text", autocomplete: "off", autocapitalize: "none", spellcheck: "false", required: true });
  const days = h("input", { type: "number", min: 1, max: 36500, required: true });
  const status = h("div");
  const form = h("form", { onsubmit: async (e) => {
    e.preventDefault();
    status.replaceChildren();
    const request = { refs: refs.value.trim(), days: Number(days.value) };
    if (!confirm(t("retention_ask", request))) return;
    try {
      await call("add_retention", request);
      retention();
    } catch (err) {
      status.replaceChildren(errorMessage(err));
    }
  } },
    ...field(t("retention_refs"), refs, t("retention_refs_hint")),
    ...field(t("retention_days"), days, t("retention_days_hint")),
    h("div", { class: "actions" }, h("button", { type: "submit", class: "primary" }, t("retention_add"))),
  );
  const rows = rules.map((r) => h("tr", {},
    h("td", { class: "nowrap" }, r.refs),
    h("td", { class: "num" }, t("report_days", { days: r.days })),
    h("td", {}, when({ seconds: r.added }))));
  show(
    h("h1", {}, t("retention_heading")),
    h("p", { class: "muted" }, t("retention_intro")),
    rules.length
      ? h("div", { class: "card scroll" }, h("table", {},
        h("thead", {}, h("tr", {}, ...["retention_refs", "retention_days", "retention_added"].map((k) => h("th", {}, t(k))))),
        h("tbody", {}, ...rows)))
      : h("p", {}, t("retention_none")),
    h("h2", {}, t("retention_add")),
    h("div", { class: "card" }, form),
    status,
  );
}

// ---- Sharing (SHARE.md §5) ----

function share(snapshot, path) {
  showNav("shares");
  const days = h("select", {}, ...[1, 7, 30].map((n) => h("option", { value: n, selected: n === 7 }, t(`days_${n}`))));
  const status = h("div");
  const form = h("form", { onsubmit: async (e) => {
    e.preventDefault();
    status.replaceChildren();
    try {
      const made = await api("POST", "/api/shares", { snapshot: snapshot.id, path, days: Number(days.value) });
      const link = h("input", { type: "text", readonly: true, value: `${location.origin}/s/${made.secret}` });
      const copied = h("span", { class: "muted", role: "status" });
      form.replaceChildren(
        message("good", t("share_made")),
        h("div", { class: "row" }, link, h("button", { type: "button", onclick: async () => {
          link.select();
          await navigator.clipboard?.writeText(link.value).catch(() => {});
          copied.textContent = t("copied");
        } }, t("copy"))),
        copied,
      );
      link.select();
    } catch (err) {
      status.replaceChildren(errorMessage(err));
    }
  } },
    ...field(t("share_days"), days),
    h("div", { class: "actions" }, h("button", { type: "submit", class: "primary" }, t("share_button"))),
  );
  const what = path || t("share_what_all");
  show(
    h("h1", {}, t("share_heading")),
    h("div", { class: "card" }, h("p", {}, t("share_intro", { time: when(snapshot.time), what })), form),
    status,
  );
}

const until = (time) => new Date(time).toLocaleString(lang, { dateStyle: "medium", timeStyle: "short" });

async function shares() {
  showNav("shares");
  show(h("p", {}, t("loading")));
  try {
    const list = await api("GET", "/api/shares");
    const owners = user?.role === "admin";
    const rows = list.map((s) => h("tr", {},
      h("td", {}, `${s.snapshot.slice(0, 12)}:/${s.path}`),
      owners ? h("td", {}, s.owner) : null,
      h("td", {}, until(s.expires)),
      h("td", {}, h("button", { type: "button", onclick: async (e) => {
        e.target.disabled = true;
        try {
          await api("POST", `/api/shares/${s.id}/remove`);
          shares();
        } catch (err) {
          main.append(errorMessage(err));
        }
      } }, t("remove"))),
    ));
    const heads = ["shares_what", ...(owners ? ["shares_owner"] : []), "shares_until", ""];
    show(
      h("h1", {}, t("shares_heading")),
      h("p", { class: "muted" }, t("shares_intro")),
      list.length === 0
        ? h("div", { class: "card" }, h("p", {}, t("shares_empty")))
        : h("div", { class: "card scroll" }, h("table", {},
          h("thead", {}, h("tr", {}, ...heads.map((k) => h("th", {}, k ? t(k) : "")))),
          h("tbody", {}, ...rows))),
    );
  } catch (err) {
    show(h("h1", {}, t("shares_heading")), errorMessage(err));
  }
}

// What a share link shows to anyone who has it (SHARE.md §4).
async function shared(secret, path = "") {
  nav.hidden = true;
  show(h("p", {}, t("loading")));
  const base = `/api/shared/${encodeURIComponent(secret)}`;
  const download = (sub) => h("a", { class: "button", href: `${base}/file?path=${encodeURIComponent(sub)}`, download: "" }, t("download"));
  try {
    const view = await api("GET", `${base}?path=${encodeURIComponent(path)}`);
    let body;
    if (view.entry && view.entry.kind !== "dir") {
      body = h("table", {}, h("tbody", {}, h("tr", {},
        h("td", {}, `📄 ${view.entry.name}`),
        h("td", { class: "num" }, view.entry.size === null ? "" : bytes(view.entry.size)),
        h("td", {}, download(path)),
      )));
    } else {
      const parts = path.split("/").filter(Boolean);
      const crumbs = h("div", { class: "crumbs" },
        h("button", { type: "button", class: "link", onclick: () => shared(secret, "") }, view.name || "/"),
        parts.map((part, i) => [
          " / ",
          h("button", { type: "button", class: "link", onclick: () => shared(secret, parts.slice(0, i + 1).join("/")) }, part),
        ]),
      );
      const rows = view.entries.map((e) => {
        const full = path ? `${path}/${e.name}` : e.name;
        const name = e.kind === "dir"
          ? h("button", { type: "button", class: "link", onclick: () => shared(secret, full) }, `📁 ${e.name}`)
          : `${e.kind === "symlink" ? "🔗" : "📄"} ${e.name}`;
        return h("tr", {},
          h("td", {}, name),
          h("td", { class: "num" }, e.size === null ? "" : bytes(e.size)),
          h("td", {}, when(e.mtime)),
          h("td", {}, e.kind === "file" ? download(full) : null),
        );
      });
      body = [crumbs, rows.length ? h("table", {}, h("tbody", {}, rows)) : h("p", { class: "muted" }, t("empty_folder"))];
    }
    show(
      h("h1", {}, t("shared_heading")),
      h("p", { class: "muted" }, t("shared_intro", { time: when(view.time), until: until(view.expires) })),
      h("div", { class: "card" }, body),
    );
  } catch (err) {
    show(h("h1", {}, t("shared_heading")), err.kind === "not_found" ? message("bad", t("shared_gone")) : errorMessage(err));
  }
}

// ---- Activity (AUDIT.md §4) ----

async function activity() {
  showNav("activity");
  show(h("p", {}, t("loading")));
  try {
    const events = await api("GET", "/api/audit?limit=200");
    const rows = events.map((e) => h("tr", {},
      h("td", {}, new Date(e.time).toLocaleString(lang, { dateStyle: "medium", timeStyle: "medium" })),
      h("td", {}, e.user || "—"),
      h("td", {}, e.event),
      h("td", { class: e.outcome === "ok" ? "" : "bad" }, e.outcome),
      h("td", { class: "muted" }, e.detail ? JSON.stringify(e.detail) : "", e.from ? ` (${e.from})` : ""),
    ));
    show(
      h("h1", {}, t("activity_heading")),
      h("p", { class: "muted" }, t("activity_intro")),
      events.length === 0
        ? h("div", { class: "card" }, h("p", {}, t("activity_empty")))
        : h("div", { class: "card scroll" }, h("table", {},
          h("thead", {}, h("tr", {}, ...["activity_time", "activity_user", "activity_event", "activity_outcome", "details"].map((k) => h("th", {}, t(k))))),
          h("tbody", {}, ...rows))),
    );
  } catch (err) {
    show(h("h1", {}, t("activity_heading")), errorMessage(err));
  }
}

// ---- Users (USERS.md §4) ----

const lines = (text) => text.split(/[\n,]/).map((s) => s.trim()).filter(Boolean);

async function users() {
  showNav("users");
  show(h("p", {}, t("loading")));
  try {
    const { users: list, folders, oidc: provider } = await api("GET", "/api/users");
    const signsIn = (u) => [u.password ? t("users_password") : null, u.oidc ? `${t("users_provider")}: ${u.oidc}` : null].filter(Boolean).join(", ");
    const rows = list.map((u) => h("tr", {},
      h("td", { class: "nowrap" }, u.name),
      h("td", { class: "nowrap" }, t(`role_${u.role}`)),
      h("td", {}, signsIn(u)),
      h("td", {}, u.role === "admin" ? t("users_all") : u.backups.join(", ")),
      h("td", {}, (u.folders.length ? u.folders : u.role === "admin" ? folders : []).join(", ")),
      h("td", {},
        h("button", { type: "button", onclick: () => userForm(u, folders, provider) }, t("users_change")),
        u.name === user.name ? null : h("button", { type: "button", onclick: async (e) => {
          if (!confirm(t("users_remove_ask", { name: u.name }))) return;
          e.target.disabled = true;
          try {
            await api("POST", `/api/users/${encodeURIComponent(u.name)}/remove`);
            users();
          } catch (err) {
            main.append(errorMessage(err));
          }
        } }, t("remove"))),
    ));
    show(
      h("h1", {}, t("users_heading")),
      h("p", { class: "muted" }, t("users_intro")),
      h("div", { class: "card scroll" }, h("table", {},
        h("thead", {}, h("tr", {}, ...["user_name", "users_role", "users_signs_in", "users_backups", "users_folders", ""].map((k) => h("th", {}, k ? t(k) : "")))),
        h("tbody", {}, ...rows))),
      h("div", { class: "actions" }, h("button", { type: "button", class: "primary", onclick: () => userForm(null, folders, provider) }, t("users_add"))),
    );
  } catch (err) {
    show(h("h1", {}, t("users_heading")), errorMessage(err));
  }
}

// Adds a user, or changes `existing`.
function userForm(existing, folders, provider) {
  const name = h("input", { type: "text", autocomplete: "off", autocapitalize: "none", spellcheck: "false", required: true, value: existing?.name || "" });
  const role = h("select", {}, ...["admin", "member", "reader"].map((r) => h("option", { value: r, selected: (existing?.role || "member") === r }, t(`role_${r}`))));
  const password = h("input", { type: "password", autocomplete: "new-password", minlength: 8 });
  const identity = h("input", { type: "text", autocomplete: "off", spellcheck: "false", value: existing?.oidc || "" });
  const backups = h("textarea", { rows: 3, spellcheck: "false" }, (existing?.backups || []).join("\n"));
  const chosen = h("textarea", { rows: 3, spellcheck: "false" }, (existing?.folders || []).join("\n"));
  const noPassword = existing?.password ? h("input", { type: "checkbox" }) : null;
  const status = h("div");
  const form = h("form", { onsubmit: async (e) => {
    e.preventDefault();
    status.replaceChildren();
    const request = {
      role: role.value,
      password: password.value,
      no_password: noPassword?.checked === true,
      oidc: identity.value,
      backups: lines(backups.value),
      folders: lines(chosen.value),
    };
    try {
      if (existing) await api("POST", `/api/users/${encodeURIComponent(existing.name)}`, request);
      else await api("POST", "/api/users", { ...request, name: name.value.trim() });
      users();
    } catch (err) {
      status.replaceChildren(errorMessage(err));
    }
  } },
    existing ? null : field(t("user_name"), name, t("users_name_hint")),
    ...field(t("users_role"), role, t("users_role_hint")),
    ...field(t("password"), password, existing ? t("users_password_keep") : t("users_password_hint")),
    noPassword ? h("label", {}, noPassword, " ", t("users_no_password")) : null,
    provider ? field(t("users_provider"), identity, t("users_provider_hint")) : null,
    ...field(t("users_backups"), backups, t("users_backups_hint")),
    ...field(t("users_folders"), chosen, t("users_folders_hint", { folders: folders.join(", ") })),
    h("div", { class: "actions" },
      h("button", { type: "submit", class: "primary" }, existing ? t("users_save") : t("users_add")),
      h("button", { type: "button", onclick: users }, t("cancel"))),
  );
  showNav("users");
  show(h("h1", {}, existing ? t("users_change_heading", { name: existing.name }) : t("users_add_heading")), h("div", { class: "card" }, form), status);
  (existing ? role : name).focus();
}

// ---- Signing in (WEB.md §4) ----

function signIn(error = null) {
  nav.hidden = true;
  const name = h("input", { type: "text", autocomplete: "username", autocapitalize: "none", spellcheck: "false", required: true });
  const password = h("input", { type: "password", autocomplete: "current-password", required: true });
  const status = h("div");
  const form = h("form", { onsubmit: async (e) => {
    e.preventDefault();
    status.replaceChildren();
    try {
      await api("POST", "/api/login", { name: name.value.trim(), password: password.value });
      begin();
    } catch (err) {
      const text = err.kind === "key" ? t("wrong_password") : err.kind === "locked" ? t("too_many_tries") : null;
      status.replaceChildren(text ? message("bad", text) : errorMessage(err));
      password.select();
    }
  } },
    ...field(t("user_name"), name),
    ...field(t("password"), password),
    h("div", { class: "actions" }, h("button", { type: "submit", class: "primary" }, t("sign_in_button"))),
  );
  const provider = oidc
    ? h("div", { class: "card" },
      h("p", { class: "muted" }, t("oidc_intro")),
      h("div", { class: "actions" }, h("a", { class: "button primary", href: "/api/oidc/start" }, t("oidc_button"))))
    : null;
  if (error) status.replaceChildren(message("bad", error === "key" ? t("oidc_refused") : t("oidc_failed")));
  show(h("h1", {}, t("sign_in_heading")), provider, h("div", { class: "card" }, h("p", { class: "muted" }, t("sign_in_intro")), form), status);
  name.focus();
}

async function signOut() {
  await api("POST", "/api/logout").catch(() => {});
  signIn();
}

// ---- Begin ----

async function begin() {
  if (location.pathname.startsWith("/s/")) return shared(decodeURIComponent(location.pathname.slice(3)));
  const session = await api("GET", "/api/session").catch(() => null);
  mode = session?.mode || "local";
  user = session?.user || null;
  oidc = session?.oidc === true;
  if (!session?.signed_in) return signIn(oidcError);
  const state = await api("GET", "/api/state").catch(() => null);
  if (state?.open) opened(state.open.path);
  else start();
}

begin();
