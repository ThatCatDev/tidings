// Service worker for the web notification test page. Chrome only gives
// notifications action buttons and reply fields when they come from a
// service worker; clicks, replies and closes arrive here and are passed on
// to the page's log.

self.addEventListener("install", () => self.skipWaiting());
self.addEventListener("activate", (event) => event.waitUntil(self.clients.claim()));

async function tell(message) {
  const pages = await self.clients.matchAll({ type: "window", includeUncontrolled: true });
  for (const page of pages) page.postMessage(message);
}

self.addEventListener("notificationclick", (event) => {
  const n = event.notification;
  event.waitUntil(
    tell({
      kind: "click",
      title: n.title,
      tag: n.tag,
      action: event.action,
      // null unless the action was a text (reply) action.
      reply: event.reply ?? null,
    }),
  );
  n.close();
});

self.addEventListener("notificationclose", (event) => {
  event.waitUntil(tell({ kind: "close", title: event.notification.title, tag: event.notification.tag }));
});
