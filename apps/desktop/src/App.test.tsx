import { afterEach, describe, expect, it } from "vitest";
import { act, cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { setApi } from "./api";
import { mockApi } from "./mock";
import { App } from "./App";
import { TrayPopover } from "./components/TrayPopover";
import { useStore } from "./store";

afterEach(cleanup);

describe("dashboard against the mock daemon", () => {
  it("groups tunnels, and asks before starting a production one", async () => {
    const calls: string[] = [];
    const base = mockApi({ configExists: true });
    setApi({ ...base, act: async (a, s) => (calls.push(`${a} ${s.names.join(",")}`), base.act(a, s)) });
    useStore.setState({ phase: "loading", tunnels: null, confirm: null });
    render(<App />);
    await screen.findByText("staging-db");
    expect(screen.getByText("No profile")).toBeTruthy();
    expect(screen.getByText("production")).toBeTruthy();
    expect(screen.getByText("2 running, 1 connecting, 1 failed")).toBeTruthy();

    await act(async () => fireEvent.click(screen.getByRole("button", { name: "Start prod-db" })));
    const dialog = await screen.findByRole("alertdialog");
    expect(within(dialog).getByText("Start a production tunnel?")).toBeTruthy();
    expect(calls).toEqual([]);
    await act(async () => fireEvent.click(within(dialog).getByRole("button", { name: "Start prod-db" })));
    await waitFor(() => expect(calls).toEqual(["start prod-db"]));

    await act(async () => fireEvent.click(screen.getByRole("button", { name: "Stop staging-db" })));
    await waitFor(() => expect(calls).toEqual(["start prod-db", "stop staging-db"]), { timeout: 2000 });
  });

  it("shows the tunnels when the daemon connected before the window was listening", async () => {
    // The app's watcher connects as soon as the app starts. With a daemon
    // already running, its one `connected` event fires before the web view has
    // registered its listener, and Tauri drops it.
    const base = mockApi({ configExists: true });
    setApi({ ...base, onDaemon: async () => () => {} });
    useStore.setState({ phase: "loading", tunnels: null, config: null, view: "tunnels" });
    render(<App />);
    expect(await screen.findByText("staging-db")).toBeTruthy();
    expect(screen.queryByText("Waiting for the daemon")).toBeNull();
  });
  it("the tray popover lists tunnels with switches and asks before starting production", async () => {
    const calls: string[] = [];
    const base = mockApi({ configExists: true });
    setApi({ ...base, act: async (a, s) => (calls.push(`${a} ${s.names.join(",")}`), base.act(a, s)) });
    useStore.setState({ phase: "loading", tunnels: null, confirm: null });
    render(<TrayPopover />);
    await screen.findByText("staging-db");
    await act(async () => fireEvent.click(screen.getByRole("switch", { name: "Stop staging-db" })));
    await waitFor(() => expect(calls).toEqual(["stop staging-db"]));
    await act(async () => fireEvent.click(screen.getByRole("switch", { name: "Start prod-db" })));
    expect(await screen.findByRole("alertdialog")).toBeTruthy();
    expect(calls).toEqual(["stop staging-db"]);
    act(() => useStore.setState({ confirm: null }));
  });
  it("runs commands from the palette and moves through the list with j and k", async () => {
    const calls: string[] = [];
    const base = mockApi({ configExists: true });
    setApi({ ...base, act: async (a, s) => (calls.push(`${a} ${s.names.join(",")}`), base.act(a, s)) });
    useStore.setState({ phase: "loading", tunnels: null, confirm: null, open: null, view: "tunnels" });
    render(<App />);
    await screen.findByText("staging-db");
    await act(async () => fireEvent.keyDown(window, { key: "k", metaKey: true }));
    const input = await screen.findByRole("combobox", { name: "Search commands" });
    fireEvent.change(input, { target: { value: "stop staging" } });
    expect(screen.getAllByRole("option")[0]?.textContent).toContain("Stop staging-db");
    await act(async () => fireEvent.keyDown(input, { key: "Enter" }));
    await waitFor(() => expect(calls).toEqual(["stop staging-db"]));
    await waitFor(() => expect(screen.queryByRole("combobox")).toBeNull());
    await act(async () => fireEvent.keyDown(window, { key: "j" }));
    expect(useStore.getState().open).toBe("staging-db");
    await act(async () => fireEvent.keyDown(window, { key: "j" }));
    expect(useStore.getState().open).toBe("staging-redis");
    act(() => useStore.setState({ open: null }));
  });
  it("shows the first-run screen without a config", async () => {
    setApi(mockApi({ configExists: false }));
    useStore.setState({ phase: "loading", tunnels: null, config: null });
    render(<App />);
    expect(await screen.findByText("Set up your tunnels")).toBeTruthy();
    expect(screen.getByRole("button", { name: "Start from the example config" })).toBeTruthy();
    expect(screen.getByRole("img", { name: "The Tunwarden warden, waving" })).toBeTruthy();
  });

  it("shows the daemon's validation error on the field it names", async () => {
    setApi(mockApi({ configExists: true }));
    useStore.setState({ phase: "loading", tunnels: null, config: null, editing: null, view: "tunnels" });
    render(<App />);
    await screen.findByText("staging-db");
    await act(async () => fireEvent.click(screen.getByRole("button", { name: "Add tunnel" })));
    const dialog = await screen.findByRole("dialog");
    fireEvent.change(within(dialog).getByLabelText("Name"), { target: { value: "cache" } });
    await act(async () => fireEvent.click(within(dialog).getByRole("button", { name: "SSH local" })));
    await act(async () => fireEvent.click(within(dialog).getByRole("button", { name: "Add tunnel" })));
    expect(await within(dialog).findByText(/a ssh-local tunnel needs `host`/)).toBeTruthy();
    expect(within(dialog).getByLabelText("SSH host").getAttribute("aria-invalid")).toBe("true");
  });

  it("logs in from the app and says which tunnels restart", async () => {
    setApi(mockApi({ configExists: true }));
    useStore.setState({ phase: "loading", tunnels: null, config: null, editing: null, loggingIn: null, view: "logins" });
    render(<App />);
    const row = (await screen.findAllByText("google")).find((e) => e.closest("tr"))!.closest("tr")!;
    await act(async () => fireEvent.click(within(row).getByRole("button", { name: "Log in" })));
    const dialog = await screen.findByRole("dialog");
    expect(await within(dialog).findByText("You are now logged in as [me@example.com].", {}, { timeout: 3000 })).toBeTruthy();
    expect(await within(dialog).findByText("Logged in. Restarting prod-api.")).toBeTruthy();
  });
});
