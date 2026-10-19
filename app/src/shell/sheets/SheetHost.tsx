import { useShell } from "../context";
import { ExportSheet } from "../messages/ExportSheet";
import { OperationHost } from "../operate/OperationHost";
import { ReportSheet } from "../operate/ReportSheet";
import { ExplorerOptionsSheet } from "./ExplorerOptionsSheet";
import { ProfileSheet } from "./ProfileSheet";
import { ProfilesSheet } from "./ProfilesSheet";
import { SettingsSheet } from "./SettingsSheet";
import { UpdateSheet } from "./UpdateSheet";

/** Renders the open side sheet, if any. */
export function SheetHost() {
  const { sheet } = useShell();
  if (!sheet) return null;
  switch (sheet.kind) {
    case "profile":
      return <ProfileSheet key={sheet.profileId ?? `new-${sheet.auth ?? ""}`} profileId={sheet.profileId} initialAuth={sheet.auth} />;
    case "profiles":
      return <ProfilesSheet />;
    case "settings":
      return <SettingsSheet />;
    case "update":
      return <UpdateSheet />;
    case "explorer-options":
      return <ExplorerOptionsSheet />;
    case "export":
      // Another export opened while one is up starts from its own choices.
      return <ExportSheet key={JSON.stringify(sheet)} {...sheet} />;
    case "operation":
      return <OperationHost key={JSON.stringify(sheet)} seed={sheet} />;
    case "report":
      return <ReportSheet key={sheet.jobId} jobId={sheet.jobId} />;
    default:
      return null;
  }
}
