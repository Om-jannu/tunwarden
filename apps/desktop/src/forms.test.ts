import { describe, expect, it } from "vitest";
import { invalidFields, loginFields, loginForm, parseEnv, profileFields, profileForm, tunnelFields, tunnelForm } from "./forms";
import { sampleEntries } from "./mock";

describe("tunnel forms", () => {
  it("round-trips an entry and keeps fields the form does not show", () => {
    const old = sampleEntries().tunnels.find((t) => t.name === "staging-redis")!;
    const f = tunnelForm(old);
    expect(f.kind).toBe("ssh-local");
    expect(f.forwards[0]).toMatchObject({ local_port: "6380", remote_host: "redis.internal", remote_port: "6379" });
    const out = tunnelFields(f, old);
    expect(out).toEqual({ type: "ssh-local", host: "bastion", profile: "staging", groups: ["daily"], max_retries: 20, forwards: [{ local_port: 6380, remote_host: "redis.internal", remote_port: 6379 }] });
  });

  it("drops fields that no longer apply when the type changes", () => {
    const old = sampleEntries().tunnels.find((t) => t.name === "staging-redis")!;
    const f = { ...tunnelForm(old), kind: "command" as const, command: "kubectl port-forward svc/redis 6380:6379", groups: "", profile: "" };
    const out = tunnelFields(f, old);
    expect(out).toEqual({ command: "kubectl port-forward svc/redis 6380:6379", max_retries: 20, forwards: [{ local_port: 6380 }] });
  });

  it("builds a new SSH tunnel and checks ports", () => {
    const f = { ...tunnelForm(null), name: "db", kind: "ssh-local" as const, host: "bastion", jump: "j1, j2", port: "2222" };
    f.forwards = [{ local_port: "5433", remote_host: "db.internal", remote_port: "5432", rest: {} }];
    expect(tunnelFields(f, null)).toEqual({ type: "ssh-local", host: "bastion", port: 2222, jump: ["j1", "j2"], forwards: [{ local_port: 5433, remote_host: "db.internal", remote_port: 5432 }] });
    f.forwards[0]!.local_port = "70000";
    expect(() => tunnelFields(f, null)).toThrow("A local port must be a port from 1 to 65535");
  });

  it("finds the fields a daemon error names", () => {
    const msg = "the change would make the config invalid:\nconfig.toml:9:1: tunnels[5].host: a ssh-local tunnel needs `host`\nconfig.toml:10:1: tunnels[5].forwards[0].remote_port: missing";
    expect([...invalidFields(msg, "tunnels")]).toEqual(["host", "forwards"]);
    expect(invalidFields(msg, "profiles").size).toBe(0);
  });
});

describe("profile and login forms", () => {
  it("reads and writes environment lines", () => {
    expect(parseEnv("A=1\n# note\n\nB = two words\n")).toEqual({ A: "1", B: "two words" });
    expect(() => parseEnv("nope")).toThrow('"nope" is not KEY=value');
    const old = sampleEntries().profiles[1]!;
    const f = profileForm(old);
    expect(f).toEqual({ name: "prod", login: "google", production: true, env: "CLOUDSDK_CORE_PROJECT=example-prod-456" });
    expect(profileFields({ ...f, production: false, login: "" }, old)).toEqual({ env: { CLOUDSDK_CORE_PROJECT: "example-prod-456" } });
  });

  it("keeps expired texts one per line", () => {
    const old = sampleEntries().logins[0]!;
    const f = loginForm(old);
    expect(f.expired).toBe("Reauthentication failed");
    const out = loginFields({ ...f, expired: "Reauthentication failed\n  invalid_grant  \n" }, old);
    expect(out.expired).toEqual(["Reauthentication failed", "invalid_grant"]);
    expect(out.env).toEqual({ CLOUDSDK_CORE_ACCOUNT: "me@example.com" });
  });
});
