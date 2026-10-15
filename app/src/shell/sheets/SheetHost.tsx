import { useShell } from "../context";
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
    default:
      return null;
  }
}
