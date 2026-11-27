import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/components/ui/alert-dialog";
import { api } from "@/api";
import { useStore } from "@/store";

/** Before starting or restarting production tunnels (FR-33). */
export function ConfirmProduction() {
  const pending = useStore((s) => s.confirm);
  const act = useStore((s) => s.act);
  const cancel = useStore((s) => s.cancelConfirm);
  const verb = pending?.action === "restart" ? "Restart" : "Start";
  const names = pending?.production ?? [];
  return (
    <AlertDialog open={pending !== null} onOpenChange={(o) => !o && cancel()}>
      <AlertDialogContent>
        <AlertDialogHeader>
          <AlertDialogTitle>
            {verb} {names.length === 1 ? "a production tunnel" : `${names.length} production tunnels`}?
          </AlertDialogTitle>
          <AlertDialogDescription>
            {names.join(", ")} {names.length === 1 ? "belongs" : "belong"} to a production profile.
          </AlertDialogDescription>
        </AlertDialogHeader>
        <AlertDialogFooter>
          <AlertDialogCancel>Cancel</AlertDialogCancel>
          <AlertDialogAction onClick={() => pending && void act(pending.action, pending.selector, true)}>
            {verb} {names.length === 1 ? names[0] : "them"}
          </AlertDialogAction>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  );
}

/** Quit from the tray: tunnels belong to the daemon, so keeping them is the default (FR-10). */
export function QuitDialog() {
  const open = useStore((s) => s.quitOpen);
  const setOpen = useStore((s) => s.setQuitOpen);
  const running = useStore((s) => (s.tunnels ?? []).filter((t) => t.state !== "stopped" && t.state !== "failed").length);
  const quit = (stop: boolean) => void api().then((a) => a.quit(stop));
  return (
    <AlertDialog open={open} onOpenChange={setOpen}>
      <AlertDialogContent>
        <AlertDialogHeader>
          <AlertDialogTitle>Quit Tunwarden?</AlertDialogTitle>
          <AlertDialogDescription>
            {running > 0
              ? `${running} ${running === 1 ? "tunnel is" : "tunnels are"} running. They can keep running without the app; the CLI and the next launch still see them.`
              : "No tunnels are running."}
          </AlertDialogDescription>
        </AlertDialogHeader>
        <AlertDialogFooter>
          <AlertDialogCancel>Cancel</AlertDialogCancel>
          {running > 0 && (
            <AlertDialogAction variant="outline" onClick={() => quit(true)}>
              Stop tunnels and quit
            </AlertDialogAction>
          )}
          <AlertDialogAction onClick={() => quit(false)}>{running > 0 ? "Keep them running" : "Quit"}</AlertDialogAction>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  );
}
