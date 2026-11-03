// EB MCP: runs at document start in every frame of the hidden sign-in webview.
// e-Boekhouden reports problems with alert(); nobody can see those here, so
// they are kept for the driver to read. confirm() is answered with "no" and
// kept as well: EB MCP never agrees to something on the user's behalf.
(() => {
  const alerts = [];
  const confirms = [];
  Object.defineProperty(window, '__ebmcpDialogs', { value: { alerts, confirms }, configurable: false });
  window.alert = (message) => {
    alerts.push(String(message ?? ''));
  };
  window.confirm = (message) => {
    confirms.push(String(message ?? ''));
    return false;
  };
  window.prompt = () => null;
})();
