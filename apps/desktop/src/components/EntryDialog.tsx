import { useEffect, useState } from "react";
import { PlusIcon, XIcon } from "lucide-react";
import { Alert, AlertDescription } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { Field, FieldDescription, FieldGroup, FieldLabel, FieldSet, FieldLegend } from "@/components/ui/field";
import { Input } from "@/components/ui/input";
import { Select, SelectContent, SelectGroup, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { Spinner } from "@/components/ui/spinner";
import { Switch } from "@/components/ui/switch";
import { Textarea } from "@/components/ui/textarea";
import { ToggleGroup, ToggleGroupItem } from "@/components/ui/toggle-group";
import { toast } from "@/components/ui/toast";
import { api, asFailure, type ConfigEntries } from "@/api";
import {
  emptyForward,
  invalidFields,
  loginFields,
  loginForm,
  profileFields,
  profileForm,
  tunnelFields,
  tunnelForm,
  type Entry,
  type Kind,
  type LoginForm,
  type ProfileForm,
  type TableName,
  type TunnelForm,
} from "@/forms";
import { useStore } from "@/store";

const NOUN: Record<TableName, string> = { tunnels: "tunnel", profiles: "profile", logins: "login" };

const KINDS: { value: Kind; label: string; hint: string }[] = [
  { value: "command", label: "Command", hint: "Any command that opens a local port, such as gcloud or kubectl." },
  { value: "ssh-local", label: "SSH local", hint: "ssh -L: a local port reaches a host behind the SSH server." },
  { value: "ssh-remote", label: "SSH remote", hint: "ssh -R: a port on the server reaches a local service." },
  { value: "ssh-dynamic", label: "SOCKS", hint: "ssh -D: a local SOCKS proxy through the SSH server." },
];

type FieldProps = { bad: Set<string> };

function TunnelFields({ f, set, profiles, bad, creating }: FieldProps & { f: TunnelForm; set: (f: TunnelForm) => void; profiles: string[]; creating: boolean }) {
  const ssh = f.kind !== "command";
  const remoteHost = f.kind === "ssh-local";
  const remotePort = f.kind === "ssh-local" || f.kind === "ssh-remote";
  const profileItems = [{ label: "No profile", value: "" }, ...profiles.map((p) => ({ label: p, value: p }))];
  const setForward = (i: number, part: Partial<TunnelForm["forwards"][number]>) =>
    set({ ...f, forwards: f.forwards.map((x, j) => (j === i ? { ...x, ...part } : x)) });
  return (
    <FieldGroup>
      {creating && (
        <Field data-invalid={bad.has("name") || undefined}>
          <FieldLabel htmlFor="t-name">Name</FieldLabel>
          <Input id="t-name" value={f.name} onChange={(e) => set({ ...f, name: e.target.value })} placeholder="staging-db" aria-invalid={bad.has("name") || undefined} autoFocus />
          <FieldDescription>Letters, digits, - _ and . It names the log file too.</FieldDescription>
        </Field>
      )}
      <Field>
        <FieldLabel>Type</FieldLabel>
        <ToggleGroup value={[f.kind]} onValueChange={(v) => v[0] && set({ ...f, kind: v[0] as Kind })} variant="outline" size="sm">
          {KINDS.map((k) => (
            <ToggleGroupItem key={k.value} value={k.value}>
              {k.label}
            </ToggleGroupItem>
          ))}
        </ToggleGroup>
        <FieldDescription>{KINDS.find((k) => k.value === f.kind)?.hint}</FieldDescription>
      </Field>
      {ssh ? (
        <>
          <div className="grid grid-cols-[1fr_8rem_6rem] gap-3">
            <Field data-invalid={bad.has("host") || undefined}>
              <FieldLabel htmlFor="t-host">SSH host</FieldLabel>
              <Input id="t-host" value={f.host} onChange={(e) => set({ ...f, host: e.target.value })} placeholder="bastion" aria-invalid={bad.has("host") || undefined} />
            </Field>
            <Field data-invalid={bad.has("user") || undefined}>
              <FieldLabel htmlFor="t-user">User</FieldLabel>
              <Input id="t-user" value={f.user} onChange={(e) => set({ ...f, user: e.target.value })} placeholder="default" />
            </Field>
            <Field data-invalid={bad.has("port") || undefined}>
              <FieldLabel htmlFor="t-port">Port</FieldLabel>
              <Input id="t-port" value={f.port} onChange={(e) => set({ ...f, port: e.target.value })} placeholder="22" inputMode="numeric" />
            </Field>
          </div>
          <Field data-invalid={bad.has("jump") || undefined}>
            <FieldLabel htmlFor="t-jump">Jump hosts</FieldLabel>
            <Input id="t-jump" value={f.jump} onChange={(e) => set({ ...f, jump: e.target.value })} placeholder="jump.example.com, user@other:2222" />
            <FieldDescription>Optional, in order. A host can also be an alias from ~/.ssh/config.</FieldDescription>
          </Field>
        </>
      ) : (
        <Field data-invalid={bad.has("command") || undefined}>
          <FieldLabel htmlFor="t-command">Command</FieldLabel>
          <Textarea id="t-command" value={f.command} onChange={(e) => set({ ...f, command: e.target.value })} className="min-h-16 font-mono text-[12px]" placeholder="gcloud compute start-iap-tunnel db-1 5432 --local-host-port=127.0.0.1:5433" aria-invalid={bad.has("command") || undefined} />
          <FieldDescription>Run directly, not through a shell: no pipes, &&, or $VARS. Point it at a script if you need them.</FieldDescription>
        </Field>
      )}
      <FieldSet data-invalid={bad.has("forwards") || undefined}>
        <FieldLegend variant="label">{f.kind === "ssh-dynamic" ? "SOCKS port" : "Ports"}</FieldLegend>
        <FieldDescription>
          {f.kind === "command"
            ? "The local ports the command opens; the tunnel is ready once they listen. Leave empty to count it ready while it runs."
            : f.kind === "ssh-remote"
              ? "A port on the server, and the local port it reaches."
              : f.kind === "ssh-local"
                ? "A local port, and the host and port it reaches from the server."
                : "The local port of the SOCKS proxy."}
        </FieldDescription>
        <div className="flex flex-col gap-2">
          {f.forwards.map((x, i) => (
            <div key={i} className="flex items-center gap-2">
              {f.kind === "ssh-remote" && (
                <Input aria-label="Remote port" value={x.remote_port} onChange={(e) => setForward(i, { remote_port: e.target.value })} placeholder="server port" className="w-28" inputMode="numeric" />
              )}
              <Input aria-label="Local port" value={x.local_port} onChange={(e) => setForward(i, { local_port: e.target.value })} placeholder="local port" className="w-28" inputMode="numeric" />
              {remoteHost && (
                <Input aria-label="Remote host" value={x.remote_host} onChange={(e) => setForward(i, { remote_host: e.target.value })} placeholder="localhost" className="flex-1" />
              )}
              {remotePort && f.kind === "ssh-local" && (
                <Input aria-label="Remote port" value={x.remote_port} onChange={(e) => setForward(i, { remote_port: e.target.value })} placeholder="remote port" className="w-28" inputMode="numeric" />
              )}
              {f.forwards.length > 1 && (
                <Button variant="ghost" size="icon" aria-label="Remove this port" onClick={() => set({ ...f, forwards: f.forwards.filter((_, j) => j !== i) })}>
                  <XIcon />
                </Button>
              )}
            </div>
          ))}
          {f.kind !== "ssh-dynamic" && (
            <Button variant="ghost" size="sm" className="self-start" onClick={() => set({ ...f, forwards: [...f.forwards, emptyForward()] })}>
              <PlusIcon data-icon="inline-start" />
              Add a port
            </Button>
          )}
        </div>
      </FieldSet>
      <div className="grid grid-cols-2 gap-3">
        <Field data-invalid={bad.has("profile") || undefined}>
          <FieldLabel>Profile</FieldLabel>
          <Select items={profileItems} value={f.profile} onValueChange={(v) => set({ ...f, profile: (v as string | null) ?? "" })}>
            <SelectTrigger className="w-full">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              <SelectGroup>
                {profileItems.map((p) => (
                  <SelectItem key={p.value} value={p.value}>
                    {p.label}
                  </SelectItem>
                ))}
              </SelectGroup>
            </SelectContent>
          </Select>
        </Field>
        <Field data-invalid={bad.has("groups") || undefined}>
          <FieldLabel htmlFor="t-groups">Groups</FieldLabel>
          <Input id="t-groups" value={f.groups} onChange={(e) => set({ ...f, groups: e.target.value })} placeholder="daily" />
        </Field>
      </div>
      {!ssh && (
        <Field data-invalid={bad.has("credential_patterns") || undefined}>
          <FieldLabel htmlFor="t-cred">Credential errors</FieldLabel>
          <Textarea id="t-cred" value={f.credential_patterns} onChange={(e) => set({ ...f, credential_patterns: e.target.value })} className="min-h-12 font-mono text-[12px]" placeholder="You must be logged in to the server" />
          <FieldDescription>Optional. Output lines that mean the credentials need fixing, one per line; the tunnel then stops instead of retrying.</FieldDescription>
        </Field>
      )}
      <Field orientation="horizontal">
        <Switch id="t-auto" checked={f.auto_start} onCheckedChange={(v) => set({ ...f, auto_start: v })} />
        <FieldLabel htmlFor="t-auto">Start when the daemon starts</FieldLabel>
      </Field>
    </FieldGroup>
  );
}

function ProfileFields({ f, set, logins, bad, creating }: FieldProps & { f: ProfileForm; set: (f: ProfileForm) => void; logins: string[]; creating: boolean }) {
  const items = [{ label: "No login", value: "" }, ...logins.map((l) => ({ label: l, value: l }))];
  return (
    <FieldGroup>
      {creating && (
        <Field data-invalid={bad.has("name") || undefined}>
          <FieldLabel htmlFor="p-name">Name</FieldLabel>
          <Input id="p-name" value={f.name} onChange={(e) => set({ ...f, name: e.target.value })} placeholder="staging" autoFocus />
        </Field>
      )}
      <Field data-invalid={bad.has("login") || undefined}>
        <FieldLabel>Login</FieldLabel>
        <Select items={items} value={f.login} onValueChange={(v) => set({ ...f, login: (v as string | null) ?? "" })}>
          <SelectTrigger className="w-full">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            <SelectGroup>
              {items.map((l) => (
                <SelectItem key={l.value} value={l.value}>
                  {l.label}
                </SelectItem>
              ))}
            </SelectGroup>
          </SelectContent>
        </Select>
        <FieldDescription>When it expires, this profile's tunnels stop and come back after you log in.</FieldDescription>
      </Field>
      <Field data-invalid={bad.has("env") || undefined}>
        <FieldLabel htmlFor="p-env">Environment</FieldLabel>
        <Textarea id="p-env" value={f.env} onChange={(e) => set({ ...f, env: e.target.value })} className="min-h-20 font-mono text-[12px]" placeholder={"CLOUDSDK_CORE_PROJECT=example-staging-123\nAWS_PROFILE=work"} />
        <FieldDescription>One KEY=value per line, added for every tunnel of this profile.</FieldDescription>
      </Field>
      <Field orientation="horizontal">
        <Switch id="p-prod" checked={f.production} onCheckedChange={(v) => set({ ...f, production: v })} />
        <FieldLabel htmlFor="p-prod">Production: ask before starting its tunnels</FieldLabel>
      </Field>
    </FieldGroup>
  );
}

function LoginFields({ f, set, bad, creating }: FieldProps & { f: LoginForm; set: (f: LoginForm) => void; creating: boolean }) {
  return (
    <FieldGroup>
      {creating && (
        <Field data-invalid={bad.has("name") || undefined}>
          <FieldLabel htmlFor="l-name">Name</FieldLabel>
          <Input id="l-name" value={f.name} onChange={(e) => set({ ...f, name: e.target.value })} placeholder="google" autoFocus />
        </Field>
      )}
      <Field data-invalid={bad.has("check") || undefined}>
        <FieldLabel htmlFor="l-check">Check command</FieldLabel>
        <Input id="l-check" value={f.check} onChange={(e) => set({ ...f, check: e.target.value })} className="font-mono text-[12px]" placeholder="gcloud config config-helper --force-auth-refresh --format=none" />
        <FieldDescription>Succeeds while the login is valid. Checked every few minutes while a tunnel uses it.</FieldDescription>
      </Field>
      <Field data-invalid={bad.has("login") || undefined}>
        <FieldLabel htmlFor="l-login">Login command</FieldLabel>
        <Input id="l-login" value={f.login} onChange={(e) => set({ ...f, login: e.target.value })} className="font-mono text-[12px]" placeholder="gcloud auth login me@example.com" />
      </Field>
      <Field data-invalid={bad.has("expired") || undefined}>
        <FieldLabel htmlFor="l-expired">Texts that mean "log in again"</FieldLabel>
        <Textarea id="l-expired" value={f.expired} onChange={(e) => set({ ...f, expired: e.target.value })} className="min-h-16 font-mono text-[12px]" placeholder="Reauthentication failed" />
        <FieldDescription>One per line, ignoring case. Without any, every failed check counts as expired.</FieldDescription>
      </Field>
      <Field data-invalid={bad.has("env") || undefined}>
        <FieldLabel htmlFor="l-env">Environment</FieldLabel>
        <Textarea id="l-env" value={f.env} onChange={(e) => set({ ...f, env: e.target.value })} className="min-h-12 font-mono text-[12px]" placeholder="CLOUDSDK_CORE_ACCOUNT=me@example.com" />
      </Field>
    </FieldGroup>
  );
}

/** Add or edit a tunnel, profile or login. Saving goes through the daemon, which validates the whole config. */
export function EntryDialog() {
  const editing = useStore((s) => s.editing);
  const close = () => useStore.getState().edit(null);
  const [entries, setEntries] = useState<ConfigEntries | null>(null);
  const [tunnel, setTunnel] = useState<TunnelForm>(tunnelForm(null));
  const [profile, setProfile] = useState<ProfileForm>(profileForm(null));
  const [login, setLogin] = useState<LoginForm>(loginForm(null));
  const [old, setOld] = useState<Entry | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [bad, setBad] = useState<Set<string>>(new Set());
  const [saving, setSaving] = useState(false);

  useEffect(() => {
    if (!editing) return;
    setError(null);
    setBad(new Set());
    void api().then(async (a) => {
      const all = await a.configEntries();
      setEntries(all);
      const found = editing.name ? ((all[editing.table] as Entry[]).find((e) => e.name === editing.name) ?? null) : null;
      setOld(found);
      setTunnel(tunnelForm(editing.table === "tunnels" ? found : null));
      setProfile(profileForm(editing.table === "profiles" ? found : null));
      setLogin(loginForm(editing.table === "logins" ? found : null));
    });
  }, [editing]);

  if (!editing) return null;
  const { table } = editing;
  const creating = editing.name === null;
  const noun = NOUN[table];
  const name = editing.name ?? (table === "tunnels" ? tunnel.name : table === "profiles" ? profile.name : login.name).trim();

  const save = async () => {
    setSaving(true);
    setError(null);
    setBad(new Set());
    try {
      if (!name) {
        setBad(new Set(["name"]));
        throw new Error(`Give the ${noun} a name.`);
      }
      const fields = table === "tunnels" ? tunnelFields(tunnel, old) : table === "profiles" ? profileFields(profile, old) : loginFields(login, old);
      const r = await (await api()).saveEntry(table, name, old, fields);
      toast.add({
        title: `${creating ? "Added" : "Saved"} ${name}`,
        description: r.restart_to_apply ? "It runs with its old settings until you restart it." : undefined,
        timeout: 3000,
      });
      if (table !== "tunnels") void useStore.getState().loadLogins();
      close();
    } catch (e) {
      const f = e instanceof Error ? { message: e.message, hint: null } : asFailure(e);
      setError(f.hint ? `${f.message}\n${f.hint}` : f.message);
      setBad((b) => (b.size ? b : invalidFields(f.message, table)));
    } finally {
      setSaving(false);
    }
  };

  return (
    <Dialog open onOpenChange={(o) => !o && close()}>
      <DialogContent className="max-h-[90vh] overflow-y-auto sm:max-w-[560px]">
        <DialogHeader>
          <DialogTitle>{creating ? `Add a ${noun}` : `Edit ${editing.name}`}</DialogTitle>
          <DialogDescription>
            {table === "tunnels"
              ? "Saved to your config file. A running tunnel keeps its old settings until you restart it."
              : table === "profiles"
                ? "Settings shared by tunnels: environment, a login, and whether it is production."
                : "Something you renew now and then, for any tool."}
          </DialogDescription>
        </DialogHeader>
        {entries === null ? (
          <div className="flex justify-center py-8 text-muted-foreground">
            <Spinner />
          </div>
        ) : table === "tunnels" ? (
          <TunnelFields f={tunnel} set={setTunnel} profiles={entries.profiles.map((p) => String(p.name))} bad={bad} creating={creating} />
        ) : table === "profiles" ? (
          <ProfileFields f={profile} set={setProfile} logins={entries.logins.map((l) => String(l.name))} bad={bad} creating={creating} />
        ) : (
          <LoginFields f={login} set={setLogin} bad={bad} creating={creating} />
        )}
        {error && (
          <Alert variant="destructive">
            <AlertDescription className="font-mono text-[12px] whitespace-pre-wrap">{error}</AlertDescription>
          </Alert>
        )}
        <DialogFooter>
          <Button variant="outline" onClick={close}>
            Cancel
          </Button>
          <Button onClick={() => void save()} disabled={saving}>
            {saving && <Spinner data-icon="inline-start" />}
            {creating ? `Add ${noun}` : "Save"}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
