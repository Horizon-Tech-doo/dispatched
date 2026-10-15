/** One plain sentence and a next step for a failed namespace listing; the raw message goes beneath it. */
export function describeListError(kind: string, host: string): string {
  switch (kind) {
    case "network":
    case "timeout":
      return `The connection to ${host} was interrupted. Check your network or VPN, then retry.`;
    case "forbidden":
      return `This identity isn't allowed to list ${host}. Grant it Manage rights or pin entities on the connection.`;
    default:
      return `Something went wrong listing ${host}. Retry, or check the activity log for details.`;
  }
}
