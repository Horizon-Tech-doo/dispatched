/** What went wrong with the copy, in plain words; the engine's own sentence stays as the detail. */
export function copyFailureBody(message: string): string {
  if (/only works for/.test(message)) return "This connection's string is scoped to another entity, so its key can't open this one. Use a connection string for the whole namespace.";
  if (/no stored connection string/.test(message)) return "Edit the connection and paste the connection string again.";
  if (/not a valid entity name/.test(message)) return "This entity's name can't go into a connection string. Copy the URL instead.";
  return "Try again. If it keeps failing, the detail below says why.";
}
