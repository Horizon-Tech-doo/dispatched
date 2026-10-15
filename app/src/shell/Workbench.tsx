import { Shell } from "./Shell";
import { NamespaceTree } from "./tree/NamespaceTree";

/** Fills the shell's slots. Tasks 15 and 17 add the workspace and drawer here. */
export function Workbench() {
  return <Shell tree={<NamespaceTree />} />;
}
