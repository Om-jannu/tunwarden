import { useEffect } from "react";
import { Spinner } from "@/components/ui/spinner";
import { Toaster } from "@/components/ui/toast";
import { TunnelsView } from "@/components/Dashboard";
import { ConfirmProduction, QuitDialog } from "@/components/Dialogs";
import { EntryDialog } from "@/components/EntryDialog";
import { FirstRun } from "@/components/FirstRun";
import { CommandPalette } from "@/components/CommandPalette";
import { Sidebar } from "@/components/Shell";
import { ActivityView, DoctorView, LoginRunDialog, LoginsView, SettingsView } from "@/components/Views";
import { useStore } from "@/store";

function Main() {
  const view = useStore((s) => s.view);
  return (
    <div className="flex h-dvh">
      <Sidebar />
      <div className="flex min-w-0 flex-1 flex-col bg-background">
        {view === "tunnels" ? (
          <TunnelsView />
        ) : (
          <main className="min-h-0 flex-1 overflow-auto">
            {view === "logins" && <LoginsView />}
            {view === "activity" && <ActivityView />}
            {view === "doctor" && <DoctorView />}
            {view === "settings" && <SettingsView />}
          </main>
        )}
      </div>
    </div>
  );
}

export function App() {
  const phase = useStore((s) => s.phase);
  useEffect(() => {
    void useStore.getState().start();
  }, []);
  return (
    <Toaster>
      {phase === "loading" && (
        <div className="flex h-dvh items-center justify-center text-muted-foreground">
          <Spinner />
        </div>
      )}
      {phase === "first-run" && <FirstRun />}
      {phase === "ready" && <Main />}
      <EntryDialog />
      <LoginRunDialog />
      <ConfirmProduction />
      <QuitDialog />
      {phase === "ready" && <CommandPalette />}
    </Toaster>
  );
}
