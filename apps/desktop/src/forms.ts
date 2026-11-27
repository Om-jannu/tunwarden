// Turning config entries into form state and back (M3b). Fields the forms do
// not show (retries, readiness, bind addresses ...) are kept as they are.

export type Entry = Record<string, unknown>;
export type TableName = "tunnels" | "profiles" | "logins";
export type Kind = "command" | "ssh-local" | "ssh-remote" | "ssh-dynamic";

export type Forward = { local_port: string; remote_host: string; remote_port: string; rest: Entry };

export type TunnelForm = {
  name: string;
  kind: Kind;
  command: string;
  host: string;
  user: string;
  port: string;
  jump: string;
  forwards: Forward[];
  profile: string;
  groups: string;
  auto_start: boolean;
  credential_patterns: string;
};

export type ProfileForm = { name: string; login: string; production: boolean; env: string };
export type LoginForm = { name: string; check: string; login: string; expired: string; env: string };

const str = (v: unknown) => (typeof v === "string" ? v : v === undefined || v === null ? "" : String(v));
const list = (v: unknown) => (Array.isArray(v) ? v.map(str) : []);

/** `KEY=value` lines to a map, and back. Blank lines and `#` comments are skipped. */
export function parseEnv(text: string): Record<string, string> {
  const env: Record<string, string> = {};
  for (const raw of text.split("\n")) {
    const line = raw.trim();
    if (!line || line.startsWith("#")) continue;
    const i = line.indexOf("=");
    if (i <= 0) throw new Error(`"${line}" is not KEY=value`);
    env[line.slice(0, i).trim()] = line.slice(i + 1).trim();
  }
  return env;
}

export function formatEnv(v: unknown): string {
  if (!v || typeof v !== "object") return "";
  return Object.entries(v as Record<string, unknown>)
    .map(([k, x]) => `${k}=${str(x)}`)
    .join("\n");
}

const lines = (text: string) => text.split("\n").map((l) => l.trim()).filter(Boolean);
const words = (text: string) => text.split(/[,\s]+/).map((w) => w.trim()).filter(Boolean);

export function emptyForward(): Forward {
  return { local_port: "", remote_host: "", remote_port: "", rest: {} };
}

export function tunnelForm(e: Entry | null): TunnelForm {
  const forwards = (Array.isArray(e?.forwards) ? (e.forwards as Entry[]) : []).map((f) => {
    const { local_port, remote_host, remote_port, ...rest } = f;
    return { local_port: str(local_port), remote_host: str(remote_host), remote_port: str(remote_port), rest };
  });
  return {
    name: str(e?.name),
    kind: ((e?.type as Kind | undefined) ?? "command"),
    command: str(e?.command),
    host: str(e?.host),
    user: str(e?.user),
    port: str(e?.port),
    jump: list(e?.jump).join(", "),
    forwards: forwards.length ? forwards : [emptyForward()],
    profile: str(e?.profile),
    groups: list(e?.groups).join(", "),
    auto_start: e?.auto_start === true,
    credential_patterns: list(e?.credential_patterns).join("\n"),
  };
}

/** Sets `key` to `value`, or removes it when the value is empty. */
function put(out: Entry, key: string, value: unknown) {
  const empty = value === "" || value === false || value === undefined || (Array.isArray(value) && value.length === 0);
  if (empty) delete out[key];
  else out[key] = value;
}

function port(text: string, what: string): number | undefined {
  if (!text.trim()) return undefined;
  const n = Number(text);
  if (!Number.isInteger(n) || n < 1 || n > 65535) throw new Error(`${what} must be a port from 1 to 65535`);
  return n;
}

/** The entry to save: the old entry with the form's fields applied. */
export function tunnelFields(f: TunnelForm, old: Entry | null): Entry {
  const out: Entry = { ...(old ?? {}) };
  delete out.name;
  put(out, "type", f.kind === "command" ? "" : f.kind);
  const ssh = f.kind !== "command";
  put(out, "command", ssh ? "" : f.command.trim());
  put(out, "host", ssh ? f.host.trim() : "");
  put(out, "user", ssh ? f.user.trim() : "");
  put(out, "port", ssh ? port(f.port, "The SSH port") : undefined);
  put(out, "jump", ssh ? words(f.jump) : []);
  put(out, "credential_patterns", ssh ? [] : lines(f.credential_patterns));
  const remote = f.kind === "ssh-local";
  const remoteOnly = f.kind === "ssh-remote";
  const forwards = f.forwards
    .filter((x) => x.local_port.trim() || x.remote_port.trim())
    .map((x) => {
      const o: Entry = { ...x.rest };
      put(o, "local_port", port(x.local_port, "A local port"));
      put(o, "remote_host", remote ? x.remote_host.trim() : "");
      put(o, "remote_port", remote || remoteOnly ? port(x.remote_port, "A remote port") : undefined);
      return o;
    });
  put(out, "forwards", forwards);
  put(out, "profile", f.profile);
  put(out, "groups", words(f.groups));
  put(out, "auto_start", f.auto_start);
  return out;
}

export function profileForm(e: Entry | null): ProfileForm {
  return { name: str(e?.name), login: str(e?.login), production: e?.production === true, env: formatEnv(e?.env) };
}

export function profileFields(f: ProfileForm, old: Entry | null): Entry {
  const out: Entry = { ...(old ?? {}) };
  delete out.name;
  put(out, "login", f.login);
  put(out, "production", f.production);
  const env = parseEnv(f.env);
  put(out, "env", Object.keys(env).length ? env : undefined);
  return out;
}

export function loginForm(e: Entry | null): LoginForm {
  return { name: str(e?.name), check: str(e?.check), login: str(e?.login), expired: list(e?.expired).join("\n"), env: formatEnv(e?.env) };
}

export function loginFields(f: LoginForm, old: Entry | null): Entry {
  const out: Entry = { ...(old ?? {}) };
  delete out.name;
  put(out, "check", f.check.trim());
  put(out, "login", f.login.trim());
  put(out, "expired", lines(f.expired));
  const env = parseEnv(f.env);
  put(out, "env", Object.keys(env).length ? env : undefined);
  return out;
}

/**
 * The fields a daemon error names, such as `tunnels[3].host` or
 * `profiles[0].env`, so the form can mark them.
 */
export function invalidFields(message: string, table: TableName): Set<string> {
  const out = new Set<string>();
  const re = new RegExp(`${table}\\[\\d+\\]\\.([a-z_]+)`, "g");
  for (const m of message.matchAll(re)) out.add(m[1]!);
  return out;
}
