"use client";

import { useCallback, useEffect, useReducer, useRef } from "react";
import { portal, wardenLantern, wardenStand } from "@/art/pixels";
import { Pixels } from "./Pixels";

// The landing page's demo: three tunnels on a signal-box diagram, worked by
// levers. It runs in the browser; the states, timings and messages follow
// what the daemon does and prints (see .impeccable/capture/).

type State = "running" | "reconnecting" | "starting" | "failed";
type Tunnel = { name: string; local: string; via: string; prod?: boolean; state: State };
type Line = { time: string; text: string };
type Box = {
  tunnels: Tunnel[];
  login: "valid" | "expired";
  busy: "drop" | "login" | null;
  register: Line[];
};
type Action =
  | { type: "state"; names: string[]; state: State }
  | { type: "login"; value: Box["login"] }
  | { type: "busy"; value: Box["busy"] }
  | { type: "log"; lines: string[] };

const initial: Box = {
  tunnels: [
    { name: "staging-db", local: "127.0.0.1:5433", via: "gcloud IAP", state: "running" },
    { name: "bastion-redis", local: "127.0.0.1:6380", via: "ssh", state: "running" },
    { name: "prod-api", local: "127.0.0.1:8080", via: "gcloud IAP", prod: true, state: "running" },
  ],
  login: "valid",
  busy: null,
  register: [],
};

function reduce(box: Box, a: Action): Box {
  switch (a.type) {
    case "state":
      return {
        ...box,
        tunnels: box.tunnels.map((t) => (a.names.includes(t.name) ? { ...t, state: a.state } : t)),
      };
    case "login":
      return { ...box, login: a.value };
    case "busy":
      return { ...box, busy: a.value };
    case "log": {
      const time = new Date().toTimeString().slice(0, 8);
      const lines = a.lines.map((text) => ({ time, text }));
      return { ...box, register: [...box.register, ...lines].slice(-4) };
    }
  }
}

const words: Record<State, string> = {
  running: "running",
  reconnecting: "reconnecting",
  starting: "starting",
  failed: "login expired",
};

const affected = ["prod-api", "staging-db"];

export function SignalBox({ copy, install }: { copy: React.ReactNode; install: React.ReactNode }) {
  const [box, send] = useReducer(reduce, initial);
  const timers = useRef<number[]>([]);
  useEffect(() => () => timers.current.forEach(clearTimeout), []);

  const later = useCallback((ms: number, fn: () => void) => {
    timers.current.push(window.setTimeout(fn, ms));
  }, []);

  const redis = box.tunnels.find((t) => t.name === "bastion-redis")!;
  const canDrop = box.busy === null && redis.state === "running";
  const canExpire = box.busy === null && box.login === "valid";
  const canLogin = box.busy === null && box.login === "expired";

  function drop() {
    send({ type: "busy", value: "drop" });
    send({ type: "state", names: ["bastion-redis"], state: "reconnecting" });
    send({ type: "log", lines: ["bastion-redis  command exited with code 255", "bastion-redis  retry 1/10 in 1s"] });
    later(1000, () => {
      send({ type: "state", names: ["bastion-redis"], state: "starting" });
      send({ type: "log", lines: ["bastion-redis  reconnecting -> starting"] });
    });
    later(2100, () => {
      send({ type: "state", names: ["bastion-redis"], state: "running" });
      send({ type: "log", lines: ["bastion-redis  starting -> running"] });
      send({ type: "busy", value: null });
    });
  }

  function expire() {
    send({ type: "login", value: "expired" });
    send({ type: "state", names: affected, state: "failed" });
    send({
      type: "log",
      lines: [
        "prod-api  failed (auth expired): the google login has expired; run `tunwarden login google`",
        "staging-db  failed (auth expired): the google login has expired; run `tunwarden login google`",
      ],
    });
  }

  function login() {
    send({ type: "busy", value: "login" });
    send({ type: "login", value: "valid" });
    send({ type: "state", names: affected, state: "starting" });
    send({ type: "log", lines: ["logged in; restarting prod-api, staging-db"] });
    later(1100, () => {
      send({ type: "state", names: affected, state: "running" });
      send({ type: "log", lines: ["prod-api  starting -> running", "staging-db  starting -> running"] });
      send({ type: "busy", value: null });
    });
  }

  const say =
    box.login === "expired"
      ? "The google login expired. Pull the login lever: one login brings both back."
      : box.busy === "login"
        ? "Logged in. Bringing them back."
        : redis.state !== "running"
          ? "SSH dropped. I'm reconnecting bastion-redis."
          : "All three running. Try a lever.";

  return (
    <>
      <div className="panel">
        {copy}
        <div className="register">
          <p className="register-head">
            <span className="plate">Register</span>
            <span>Demo in your browser. Messages as the daemon writes them.</span>
          </p>
          <ol className="register-lines" aria-live="polite">
            {box.register.length === 0 ? (
              <li className="register-empty">Nothing yet. Every change of state is written here.</li>
            ) : (
              box.register.map((l, i) => (
                <li key={`${i}-${l.time}-${l.text}`}>
                  <time>{l.time}</time> {l.text}
                </li>
              ))
            )}
          </ol>
        </div>

      <div className="diagram" aria-label="Demo: three tunnels">
        <div className="warden-ledge">
          <p className="speech" aria-live="polite">
            {say}
          </p>
          {box.login === "expired" ? (
            <Pixels art={wardenLantern} lamp="#f0645a" scale={4} className="warden" label="The warden holding up a red lantern" />
          ) : box.busy !== null || redis.state !== "running" ? (
            <Pixels art={wardenLantern} lamp="#e3a43b" scale={4} className="warden" label="The warden holding up an amber lantern" />
          ) : (
            <Pixels art={wardenStand} scale={4} className="warden" label="The Tunwarden warden" />
          )}
        </div>

        <ul className="tracks">
          {box.tunnels.map((t) => (
            <li key={t.name} className="track" data-state={t.state}>
              <div className="track-label">
                <span className="track-plates">
                  <span className="plate plate-name">{t.name}</span>
                  {t.prod ? <span className="plate plate-prod">production</span> : null}
                </span>
                <span className="track-meta">
                  {t.local} · {t.via}
                </span>
                <span className="plate plate-state">{words[t.state]}</span>
              </div>
              <span className="rail" aria-hidden="true" />
              <Portal />
            </li>
          ))}
        </ul>

      </div>
      </div>

      <div className="frame">
      <div className="frame-levers" role="group" aria-label="Demo levers">
        <Lever
          colour="black"
          label="Drop the SSH connection"
          reversed={box.busy === "drop"}
          locked={canDrop ? null : "Locked while bastion-redis is not running"}
          onPull={drop}
        />
        <Lever
          colour="red"
          label="Expire the google login"
          reversed={box.login === "expired"}
          locked={canExpire ? null : box.login === "expired" ? "Locked: the login has expired" : "Locked while tunnels change"}
          onPull={expire}
        />
        <Lever
          colour="blue"
          label="tunwarden login google"
          reversed={box.busy === "login"}
          locked={canLogin ? null : "Locked until a login expires"}
          onPull={login}
          code
        />
      </div>
        <div className="frame-install">{install}</div>
      </div>
    </>
  );
}

// The portal's lamp is the same 7×7 disc the app icon and tray use; CSS gives
// it the track's state colour.
function Portal() {
  return <Pixels art={portal} scale={4} className="portal" />;
}

function Lever({
  colour,
  label,
  reversed,
  locked,
  onPull,
  code,
}: {
  colour: "black" | "red" | "blue";
  label: string;
  reversed: boolean;
  locked: string | null;
  onPull: () => void;
  code?: boolean;
}) {
  return (
    <button
      type="button"
      className="lever"
      data-colour={colour}
      data-reversed={reversed}
      aria-disabled={locked !== null}
      onClick={() => locked === null && onPull()}
    >
      <span className="lever-quadrant" aria-hidden="true">
        <span className="lever-arm" />
      </span>
      <span className="lever-badge">
        {code ? <code>{label}</code> : label}
        <span className="lever-lock">{locked ?? "Pull"}</span>
      </span>
    </button>
  );
}
