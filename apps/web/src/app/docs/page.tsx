import type { Metadata } from "next";
import Link from "next/link";
import { Footer, Nameboard } from "@/components/Chrome";
import { Terminal } from "@/components/Terminal";
import { install, repo } from "@/site";

export const metadata: Metadata = {
  title: "Docs",
  description: "Install Tunwarden, describe your tunnels in one config file, and run them: commands, logins, autostart and known limits.",
};

const contents = [
  ["install", "Install"],
  ["quick-start", "Quick start"],
  ["config", "Config"],
  ["logins", "Logins"],
  ["commands", "Commands"],
  ["autostart", "Start at login"],
  ["limits", "Known limits"],
] as const;

const quickStart = `tunwarden init                  # write an example config
tunwarden import ~/.zshrc       # or turn your tunnel aliases into entries
tunwarden doctor                # check tools, logins and ports
tunwarden start --group daily
tunwarden status
tunwarden logs staging-db -f`;

const configExample = `version = 1

[[logins]]                        # something you renew now and then
name = "google"
check = "gcloud config config-helper --force-auth-refresh --format=none"
login = "gcloud auth login me@example.com"
expired = ["Reauthentication failed"]
env = { CLOUDSDK_CORE_ACCOUNT = "me@example.com" }

[[profiles]]                      # environment, production flag and login shared by tunnels
name = "staging"
login = "google"
env = { CLOUDSDK_CORE_PROJECT = "example-staging-123" }

[[tunnels]]                       # any command that opens a local port
name = "staging-db"
profile = "staging"
command = "gcloud compute start-iap-tunnel db-1 5432 --zone=europe-west1-b --local-host-port=127.0.0.1:5433"
groups = ["daily"]
forwards = [{ local_port = 5433 }]

[[tunnels]]                       # SSH forwards are built for you, with keepalives and no prompts
name = "bastion-redis"
type = "ssh-local"
host = "bastion"
jump = ["jump.example.com"]
forwards = [{ local_port = 6380, remote_host = "redis.internal", remote_port = 6379 }]`;

const loginExample = `$ tunwarden login google
Running the google login (used by staging, prod): gcloud auth login me@example.com
...
logged in; restarting prod-api, staging-db`;

const commands: [string, string][] = [
  ["init", "Write an example config."],
  ["import", "Turn pasted tunnel commands or shell aliases into config entries."],
  ["list", "List the configured tunnels."],
  ["status", "Show each tunnel's state, port, uptime, restarts and the reason it failed."],
  ["start, stop, restart", "Take names, --group, --profile or --all."],
  ["logs", "Show a tunnel's log; -f follows it."],
  ["show-command", "Print the exact command a tunnel runs."],
  ["add, edit, remove", "Change entries without opening the file."],
  ["auth, login", "Check logins, or run one and restart the tunnels waiting for it."],
  ["trust", "Accept an SSH host key; it asks before trusting it."],
  ["doctor", "Check the config, tools, daemon, logins and ports."],
  ["daemon", "Start, stop, restart or check the background daemon."],
  ["autostart", "Start the daemon when you log in."],
  ["config-path", "Show where the config lives."],
  ["completions", "Print shell completions."],
];

export default function Docs() {
  return (
    <>
      <Nameboard current="docs" />
      <main className="docs">
        <nav className="docs-contents" aria-label="On this page">
          <p className="docs-contents-title">Rule book</p>
          <ol>
            {contents.map(([id, label]) => (
              <li key={id}>
                <a href={`#${id}`}>{label}</a>
              </li>
            ))}
          </ol>
        </nav>

        <article className="docs-body">
          <h1>Docs</h1>
          <p className="docs-intro">
            Tunwarden runs on macOS, Linux and Windows. A small background daemon owns the tunnels, so they survive
            closing the terminal. The CLI and the desktop app both talk to it. This page covers the essentials; every
            command also has <code>--help</code> with examples.
          </p>

          <h2 id="install">Install</h2>
          <p>
            See <Link href="/download/">Download</Link> for the desktop app and the one-line installers. From source,
            with Rust:
          </p>
          <Terminal text={install.source} label="Install from source" />
          <p>Tunwarden runs your own tools. Install ssh, gcloud, kubectl or whatever your tunnels use.</p>

          <h2 id="quick-start">Quick start</h2>
          <Terminal text={quickStart} label="Quick start commands" />
          <p>
            <code>tunwarden config-path</code> shows where the config lives. <code>import</code> prints the entries it
            found; add <code>--write</code> to save them.
          </p>

          <h2 id="config">Config</h2>
          <p>
            One TOML file holds three kinds of entry. <strong>Logins</strong> are things you renew now and then.{" "}
            <strong>Profiles</strong> group an environment, its production flag and its login.{" "}
            <strong>Tunnels</strong> are any command that opens a local port, or an SSH forward that Tunwarden builds for
            you (<code>ssh-local</code>, <code>ssh-remote</code>, <code>ssh-dynamic</code>).
          </p>
          <Terminal text={configExample} label="Example config.toml" />
          <p>
            Commands are never run through a shell: pipes, <code>&&</code> and <code>$VAR</code> are rejected; point
            the command at a script if you need them. Quotes group words as in a shell. On Windows a backslash is an
            ordinary character, so <code>C:\tools\gcloud.cmd</code> needs no quotes; on macOS and Linux it escapes the
            next character. The full example, with AWS and kubectl entries, is{" "}
            <a href={`${repo}/blob/main/crates/tunwarden-cli/example-config.toml`}>example-config.toml</a>.
          </p>

          <h2 id="logins">Logins</h2>
          <p>
            When a login expires, the tunnels that use it stop with “login expired” and the next step. One login renews
            it and brings them back:
          </p>
          <Terminal text={loginExample} label="Renewing a login" />
          <p>
            Tunnels in a production profile ask before they start. Pass <code>--yes</code> to confirm.
          </p>

          <h2 id="commands">Commands</h2>
          <dl className="docs-commands">
            {commands.map(([name, text]) => (
              <div key={name}>
                <dt>
                  <code>{name}</code>
                </dt>
                <dd>{text}</dd>
              </div>
            ))}
          </dl>
          <p>
            On a terminal, a command missing its tunnel or login name shows a list to pick from, and <code>start</code>{" "}
            shows a live line per tunnel. In scripts and pipes nothing asks or animates, colours are off, and a missing
            name is an error listing the choices. <code>NO_COLOR=1</code> turns colours off everywhere.
          </p>
          <p>Shell completion knows your tunnel, group, profile and login names. For zsh:</p>
          <Terminal text={`echo 'source <(tunwarden completions zsh)' >> ~/.zshrc`} label="Shell completions" />
          <p>
            Bash works the same way; fish and PowerShell are in <code>tunwarden completions --help</code>.
          </p>

          <h2 id="autostart">Start at login</h2>
          <p>
            <code>tunwarden autostart enable</code> starts the daemon when you log in, and running tunnels come back
            after a reboot. <code>disable</code> and <code>status</code> do what they say. The desktop app has the same
            switch in its settings.
          </p>

          <h2 id="limits">Known limits</h2>
          <ul>
            <li>
              A tunnel counts as healthy while its local port is listening. If the remote side dies but the port stays
              open, it can show <code>running</code> until the tool itself exits; SSH tunnels exit within 45 seconds
              through keepalives.
            </li>
            <li>
              For <code>ssh-remote</code>, the server may bind the remote port on IPv6 only when IPv4 is taken, and
              Tunwarden cannot see that.
            </li>
          </ul>
        </article>
      </main>
      <Footer />
    </>
  );
}
