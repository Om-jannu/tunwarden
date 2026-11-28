import Link from "next/link";
import { trayArch } from "@/art/pixels";
import { Footer, Nameboard } from "@/components/Chrome";
import { PrimaryInstall } from "@/components/Install";
import { Pixels } from "@/components/Pixels";
import { SignalBox } from "@/components/SignalBox";
import { Terminal } from "@/components/Terminal";
import { capturedOn, config, expired, login, prodGuard, recovered } from "@/content/transcript";

export default function Home() {
  return (
    <>
      <Nameboard />
      <main>
        <section className="box" aria-labelledby="hero-title">
          <SignalBox
            copy={
              <div className="panel-copy">
                <h1 id="hero-title">
                  Every tunnel, <span className="h1-line">signalled.</span>
                </h1>
                <p className="lede">
                  Tunwarden starts your SSH and cloud tunnels, checks that each one really listens, reconnects what
                  drops, and when a login expires it stops the tunnels with the reason and the one command that brings
                  them back.
                </p>
                <p className="panel-facts">macOS, Windows and Linux · CLI and desktop app · free and open source</p>
                <div className="install-phone">
                  <PrimaryInstall />
                </div>
              </div>
            }
            install={<PrimaryInstall />}
          />
        </section>

        <section className="section app" aria-labelledby="app-title">
          <h2 id="app-title">The same box, on your desk.</h2>
          <p className="section-lede">
            The desktop app shows every tunnel with its state and the reason it failed, and opens a tunnel&rsquo;s log
            and its last 24 hours beside the list.
          </p>
          <figure className="shot shot-video">
            <video
              controls
              playsInline
              preload="none"
              poster="/video/seven-terminals.webp"
              width={1920}
              height={1080}
              aria-label="Seven tunnel commands in seven terminals, two of them failing, fold into the seven rows of the Tunwarden window, which shows each failure's reason."
            >
              <source src="/video/seven-terminals.mp4" type="video/mp4" />
            </video>
            <figcaption>
              25 seconds, with sound. Seven tunnels in seven terminals, then the same seven in one window. The hosts are
              the example config&rsquo;s made-up ones; every error on screen is real output.
            </figcaption>
          </figure>
          <figure className="shot">
            <a href="/screens/main-dark.webp" className="shot-link" aria-label="Open the screenshot at full size">
<img
              src="/screens/main-dark.webp"
              width={1600}
              height={1098}
              alt="The Tunwarden window on macOS: tunnels grouped by profile, three failed and one running, and the running tunnel's log and 24-hour strip in a pane on the right."
              loading="lazy"
              decoding="async"
            />
</a>
            <figcaption>Tunwarden 0.1.0 on macOS, with the example config from <code>tunwarden init</code>. Its hosts are made up, so most tunnels stop or fail, each with its reason.</figcaption>
          </figure>
        </section>

        <section className="section recovery" aria-labelledby="recovery-title">
          <h2 id="recovery-title">One login, and they come back.</h2>
          <p className="section-lede">
            The same thing as the levers, in a terminal. This is real output from <code>tunwarden</code> 0.1.0, captured
            on {capturedOn} with demo tunnels and a stand-in login script.
          </p>
          <ol className="steps">
            <li>
              <p className="step-note">
                <span className="plate">Login expires</span>
                The google login expires. The two tunnels that use it stop with the reason and the next step. The SSH
                tunnel does not use it and keeps running.
              </p>
              <Terminal text={expired} label="tunwarden status after the login expired" />
            </li>
            <li>
              <p className="step-note">
                <span className="plate">One login</span>
                One login. Tunwarden runs your own login command, then restarts what it stopped.</p>
              <Terminal text={login} label="tunwarden login google" />
            </li>
            <li>
              <p className="step-note">
                <span className="plate">Back</span>
                Both are back. Nothing else was touched.
              </p>
              <Terminal text={recovered} label="tunwarden status after logging in" />
            </li>
          </ol>
        </section>

        <section className="rulebook" aria-labelledby="rules-title">
          <div className="rulebook-sheet">
            <h2 id="rules-title">Box rules</h2>
            <p className="section-lede">
              Production tunnels sit next to staging ones, so Tunwarden is strict about what it will do.
            </p>
            <dl className="rules">
              <div>
                <dt>Production asks first</dt>
                <dd>
                  Tunnels in a production profile do not start without a confirmation.
                  <Terminal text={prodGuard} label="Starting a production tunnel without confirming" />
                </dd>
              </div>
              <div>
                <dt>No shell</dt>
                <dd>
                  Commands run directly, never through a shell. Pipes, <code>&&</code> and <code>$VAR</code> are
                  rejected; point the command at a script if you need them.
                </dd>
              </div>
              <div>
                <dt>Local only</dt>
                <dd>Forwards bind to 127.0.0.1. Nothing listens on your network.</dd>
              </div>
              <div>
                <dt>Your logins stay yours</dt>
                <dd>
                  Tunwarden stores no credentials. It runs the check and login commands you already use, such as{" "}
                  <code>gcloud auth login</code>.
                </dd>
              </div>
              <div>
                <dt>Every failure says why</dt>
                <dd>A failed tunnel carries a reason and the next step, in the CLI, the app and the log.</dd>
              </div>
              <div>
                <dt>Nothing phones home</dt>
                <dd>No account, no telemetry, and no network calls of its own.</dd>
              </div>
            </dl>
          </div>
        </section>

        <section className="section tools" aria-labelledby="tools-title">
          <h2 id="tools-title">Any command that opens a port.</h2>
          <p className="section-lede">
            Tunwarden supervises the tools you already have. It is not a VPN and does not replace ssh, gcloud or kubectl;
            you describe each tunnel once, in one TOML file.
          </p>
          <ul className="tool-plates" aria-label="Examples">
            <li className="plate">ssh -L, -R, -D</li>
            <li className="plate">gcloud compute start-iap-tunnel</li>
            <li className="plate">kubectl port-forward</li>
            <li className="plate">aws ssm start-session</li>
            <li className="plate">your own script</li>
          </ul>
          <Terminal text={config} label="Example config.toml" />
          <p className="tools-after">
            Already have shell aliases? <code>tunwarden import ~/.zshrc</code> turns them into entries.{" "}
            <Link href="/docs/#config">How the config works</Link>
          </p>
        </section>

        <section className="section tray" aria-labelledby="tray-title">
          <h2 id="tray-title">The lamp follows you to the menu bar.</h2>
          <p className="section-lede">
            The desktop app lives in the tray or menu bar. Its lamp shows the worst state among your tunnels, and the app
            and <code>tunwarden status</code> read the same daemon, so they never disagree.
          </p>
          <div className="tray-layout">
            <div>
            <ul className="tray-row">
              {(
                [
                  ["running", "Running"],
                  ["connecting", "Connecting or reconnecting"],
                  ["failed", "A tunnel failed"],
                  ["idle", "Nothing running"],
                ] as const
              ).map(([state, text]) => (
                <li key={state}>
                  <TrayGlyph state={state} />
                  <span>{text}</span>
                </li>
              ))}
            </ul>
            </div>
            <figure className="shot shot-popover">
              <a href="/screens/popover-dark.webp" className="shot-link" aria-label="Open the screenshot at full size">
<img
                src="/screens/popover-dark.webp"
                width={560}
                height={746}
                alt="The Tunwarden tray popover: each tunnel with its state and a switch, and a failed tunnel's reason in red."
                loading="lazy"
                decoding="async"
              />
</a>
              <figcaption>Click the lamp: every tunnel, with a switch.</figcaption>
            </figure>
          </div>
        </section>

        <section className="close" aria-labelledby="close-title">
          <h2 id="close-title">Put your tunnels in the box.</h2>
          <PrimaryInstall />
          <p className="close-links">
            <Link href="/download/">All downloads</Link>
            <Link href="/docs/#quick-start">Quick start</Link>
          </p>
        </section>
      </main>
      <Footer />
    </>
  );
}

const trayColour = { running: "#3cc27a", connecting: "#e3a43b", failed: "#f0645a", idle: "#8c96a5" };

function TrayGlyph({ state }: { state: keyof typeof trayColour }) {
  const cells: [number, number, boolean][] = [];
  for (let dy = -2; dy <= 2; dy++)
    for (let dx = -2; dx <= 2; dx++) {
      const d = dx * dx + dy * dy;
      if (d <= 5) cells.push([8 + dx, 10 + dy, d > 2]);
    }
  return (
    <Pixels art={trayArch} scale={4} className="tray-glyph">
      {cells
        .filter(([, , ring]) => state !== "idle" || ring)
        .map(([x, y]) => (
          <rect key={`${x}-${y}`} x={x} y={y} width={1} height={1} fill={trayColour[state]} />
        ))}
    </Pixels>
  );
}
