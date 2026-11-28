import { useState } from "react";
import { wardenWave } from "@/art/pixels";
import { Pixels } from "@/components/Pixels";
import { FileTextIcon, FolderOpenIcon, UploadIcon } from "lucide-react";
import { Alert, AlertDescription } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { Field, FieldDescription, FieldGroup, FieldLabel } from "@/components/ui/field";
import { Spinner } from "@/components/ui/spinner";
import { Textarea } from "@/components/ui/textarea";
import { api, asFailure, type ImportPreview } from "@/api";
import { useStore } from "@/store";

/**
 * Shown when there is no config yet, and from Import later. Three ways in:
 * import shell aliases, start from the example, or point at an existing file.
 */
export function FirstRun() {
  const config = useStore((s) => s.config);
  const refreshConfig = useStore((s) => s.refreshConfig);
  const [text, setText] = useState("");
  const [preview, setPreview] = useState<ImportPreview | null>(null);
  const [checked, setChecked] = useState<Set<string>>(new Set());
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const fresh = !config?.exists;

  const run = async (f: () => Promise<void>) => {
    setBusy(true);
    setError(null);
    try {
      await f();
    } catch (e) {
      const x = asFailure(e);
      setError(x.hint ? `${x.message}. ${x.hint}` : x.message);
    } finally {
      setBusy(false);
    }
  };

  const makePreview = (input: string) =>
    run(async () => {
      const p = await (await api()).importPreview(input);
      setPreview(p);
      setChecked(new Set(p.tunnels.map((t) => t.name)));
    });

  const readFile = async (file: File) => {
    const content = await file.text();
    setText(content);
    await makePreview(content);
  };

  const apply = () =>
    run(async () => {
      await (await api()).importApply(text, [...checked]);
      await refreshConfig();
      useStore.setState({ phase: "ready" });
    });

  const example = () =>
    run(async () => {
      await (await api()).initExample();
      await (await api()).openConfig();
      await refreshConfig();
    });

  return (
    <div className="flex h-dvh justify-center overflow-auto bg-background px-6 py-10">
      <div className="flex w-full max-w-[640px] flex-col gap-6">
        <header className="flex flex-col gap-1">
          {fresh && <Pixels art={wardenWave} scale={4} className="mb-3" label="The Tunwarden warden, waving" />}
          <h1 className="text-[20px] font-semibold">{fresh ? "Set up your tunnels" : "Import tunnels"}</h1>
          <p className="text-muted-foreground">
            {fresh ? "Tunwarden keeps your tunnels running and tells you when a login expires. " : ""}
            Paste the tunnel commands or aliases you use today, or choose your shell profile; you pick what to add.
          </p>
        </header>

        <FieldGroup>
          <Field>
            <FieldLabel htmlFor="paste">Tunnel commands or aliases</FieldLabel>
            <Textarea
              id="paste"
              value={text}
              onChange={(e) => {
                setText(e.target.value);
                setPreview(null);
              }}
              placeholder={"alias db='gcloud compute start-iap-tunnel db-1 5432 --local-host-port=localhost:5433'\nssh -N -L 6380:redis.internal:6379 bastion"}
              className="min-h-28 font-mono text-[12px]"
            />
            <FieldDescription>Lines that are not tunnels are skipped. Nothing is written until you choose Add.</FieldDescription>
          </Field>
        </FieldGroup>
        <div className="flex gap-2">
          <Button disabled={!text.trim() || busy} onClick={() => void makePreview(text)}>
            {busy && !preview ? <Spinner data-icon="inline-start" /> : <UploadIcon data-icon="inline-start" />}
            Find tunnels
          </Button>
          <Button variant="outline" render={<label />} nativeButton={false}>
            <FileTextIcon data-icon="inline-start" />
            Choose a file…
            <input
              type="file"
              className="sr-only"
              onChange={(e) => {
                const f = e.target.files?.[0];
                if (f) void readFile(f);
              }}
            />
          </Button>
        </div>

        {error && (
          <Alert variant="destructive">
            <AlertDescription>{error}</AlertDescription>
          </Alert>
        )}

        {preview && (
          <section className="flex flex-col gap-3" aria-label="Found tunnels">
            {preview.tunnels.length === 0 ? (
              <p className="text-muted-foreground">No tunnel commands found in that text.</p>
            ) : (
              <ul className="flex flex-col divide-y rounded-md border bg-card">
                {preview.tunnels.map((t) => (
                  <li key={t.name}>
                    <label className="flex cursor-pointer items-start gap-3 px-3 py-2">
                      <Checkbox
                        className="mt-0.5"
                        checked={checked.has(t.name)}
                        onCheckedChange={(v) =>
                          setChecked((s) => {
                            const n = new Set(s);
                            if (v === true) n.add(t.name);
                            else n.delete(t.name);
                            return n;
                          })
                        }
                      />
                      <span className="flex min-w-0 flex-col">
                        <span className="flex gap-2">
                          <span className="font-medium">{t.name}</span>
                          <span className="text-muted-foreground">
                            {t.kind}
                            {t.local.length ? `, port ${t.local.join(", ")}` : ", no local port found"}
                            {t.profile ? `, profile ${t.profile}` : ""}
                          </span>
                        </span>
                        <span className="truncate font-mono text-[11.5px] text-muted-foreground">{t.summary}</span>
                      </span>
                    </label>
                  </li>
                ))}
              </ul>
            )}
            {preview.warnings.length > 0 && (
              <ul className="flex flex-col gap-0.5 text-busy">
                {preview.warnings.map((w) => (
                  <li key={w}>{w}</li>
                ))}
              </ul>
            )}
            {preview.tunnels.length > 0 && (
              <div className="flex items-center gap-3">
                <Button disabled={checked.size === 0 || busy} onClick={() => void apply()}>
                  Add {checked.size} {checked.size === 1 ? "tunnel" : "tunnels"}
                </Button>
                <span className="text-muted-foreground">They are added stopped; start them from the list.</span>
              </div>
            )}
          </section>
        )}

        <section className="flex flex-col gap-2 border-t pt-5">
          {fresh ? (
            <>
              <h2 className="font-semibold">Other ways to start</h2>
              <div className="flex flex-wrap gap-2">
                <Button variant="outline" disabled={busy} onClick={() => void example()}>
                  Start from the example config
                </Button>
                <Button variant="ghost" onClick={() => void refreshConfig()}>
                  <FolderOpenIcon data-icon="inline-start" />
                  I already have a config
                </Button>
              </div>
              <p className="text-muted-foreground">
                Tunwarden reads <code className="font-mono text-[12px] text-foreground">{config?.path}</code>. To use another file, set{" "}
                <code className="font-mono text-[12px] text-foreground">TUNWARDEN_CONFIG</code> before starting the app.
              </p>
            </>
          ) : (
            <Button variant="ghost" className="self-start" onClick={() => useStore.setState({ phase: "ready" })}>
              Back to tunnels
            </Button>
          )}
        </section>
      </div>
    </div>
  );
}
