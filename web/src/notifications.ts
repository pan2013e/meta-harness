export function browserNotificationPermission() {
  return typeof Notification === "undefined" ? "unavailable" : Notification.permission;
}

export async function showNotification(body: string, id?: string): Promise<boolean> {
  if (window.metaHarnessDesktop?.notify) return window.metaHarnessDesktop.notify(body);
  if (browserNotificationPermission() !== "granted") return false;
  const notification = new Notification("Meta-Harness", { body, tag: id, icon: "/meta-harness.svg" });
  notification.onclick = () => { window.focus(); notification.close(); };
  return true;
}
