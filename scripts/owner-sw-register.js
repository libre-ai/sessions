(() => {
  "use strict";
  if (!("serviceWorker" in navigator)) return;
  window.addEventListener("load", () => {
    navigator.serviceWorker
      .register("/app/sw.js", { scope: "/app/", updateViaCache: "none" })
      .catch((error) => console.warn("Service worker registration failed", error));
  });
})();
