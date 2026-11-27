import { useEffect } from "react";
import { Spinner } from "@/components/ui/spinner";
import { Toaster } from "@/components/ui/toast";
import { Banners, Header, TunnelsView } from "@/components/Dashboard";
import { ConfirmProduction, QuitDialog } from "@/components/Dialogs";
import { EntryDialog } from "@/components/EntryDialog";
import { FirstRun } from "@/components/FirstRun";
import { TunnelSheet } from "@/components/TunnelSheet";
import { ActivityView, DoctorView, LoginRunDialog, LoginsView, SettingsView } from "@/components/Views";
import { useStore } from "@/store";

function Main() {
  const view = useStore((s) => s.view);
  return (
    <div className="flex h-dvh flex-col">
      <Header />
      <Banners />
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
      <TunnelSheet />
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
    </Toaster>
  );
}
