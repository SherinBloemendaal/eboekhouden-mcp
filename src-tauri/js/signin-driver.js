// EB MCP: drives e-Boekhouden's own login page inside the hidden webview.
//
// Evaluated in the top window (secure.e-boekhouden.nl/bh/). The form lives
// deeper: /bh/ frameset > secure20 v1login (other origin) > inloggen.asp. The
// last one has the same origin as the top, so its document is reachable by
// walking window.frames; documents of other origins are skipped.
//
// It only fills e-Boekhouden's own fields and clicks e-Boekhouden's own
// button, so their hidden check fields and fingerprint stay theirs. It never
// reads, keeps or reports a password or code, and it sends nothing anywhere.
//
//   probe()          -> { phase, message }   phase: loading | form | code | error | app | unknown
//
// Alerts and confirms are read from what dialog-hook.js kept in each frame.
//   fill(user, pass) -> 'ok' | 'noform'
//   code(value)      -> 'ok' | 'nofield'
(() => {
  const CODE_HINT = /code|otp|totp|token|verif|2fa|tweestap|authenticat/i;
  const TWO_STEP_TEXT = /tweestaps|twee-staps|verificatiecode|authenticator|beveiligingscode/i;
  const ERROR_HINT = /error|fout|melding|alert|warn|valid|message|msg/i;
  const ERROR_TEXT = /onjuist|ongeldig|niet juist|niet correct|klopt niet|mislukt|geblokkeerd|incorrect|invalid|verlopen|te vaak/i;
  const ALREADY = /reeds ingelogd|al ingelogd/i;
  const LOGIN_PATHS = /^\/bh\/?(index\.asp|default\.asp|inloggen\.asp)?$/i;

  function documents(win, out = [], depth = 0) {
    if (depth > 6) return out;
    try {
      if (win.document) out.push(win.document);
    } catch {
      // Another origin: not ours to read.
    }
    let count = 0;
    try {
      count = win.frames.length;
    } catch {
      count = 0;
    }
    for (let i = 0; i < count; i += 1) {
      try {
        documents(win.frames[i], out, depth + 1);
      } catch {
        // A frame that went away while walking.
      }
    }
    return out;
  }

  function shown(el) {
    if (!el || el.type === 'hidden' || el.hidden || el.disabled) return false;
    const view = el.ownerDocument.defaultView;
    for (let node = el; node && node.nodeType === 1; node = node.parentElement) {
      const style = view.getComputedStyle(node);
      if (style.display === 'none' || style.visibility === 'hidden') return false;
    }
    return true;
  }

  function loginDocument(docs) {
    return docs.find((d) => d.getElementById('txtEmail') && d.getElementById('txtEmail').form) ?? null;
  }

  function codeInput(docs) {
    for (const d of docs) {
      for (const input of d.querySelectorAll('input')) {
        if (input.id === 'txtEmail' || input.type === 'password' || !['text', 'tel', 'number', ''].includes(input.type)) continue;
        const hint = `${input.id} ${input.name} ${input.getAttribute('autocomplete') ?? ''} ${input.getAttribute('placeholder') ?? ''}`;
        if ((input.getAttribute('autocomplete') === 'one-time-code' || CODE_HINT.test(hint)) && shown(input)) return input;
      }
    }
    return null;
  }

  function clean(text) {
    return String(text ?? '').replace(/\s+/g, ' ').trim().slice(0, 200);
  }

  function errorText(doc) {
    for (const el of doc.querySelectorAll('[id], [class]')) {
      const key = `${el.id} ${typeof el.className === 'string' ? el.className : ''}`;
      if (!ERROR_HINT.test(key) || !shown(el)) continue;
      const text = clean(el.textContent);
      if (text && text.length > 3 && ERROR_TEXT.test(text)) return text;
    }
    const body = clean(doc.body?.textContent);
    const match = body.match(new RegExp(`[^.!?]*(${ERROR_TEXT.source})[^.!?]*[.!?]?`, 'i'));
    return match ? clean(match[0]) : null;
  }

  function setValue(input, value) {
    const proto = input.ownerDocument.defaultView.HTMLInputElement.prototype;
    const setter = Object.getOwnPropertyDescriptor(proto, 'value').set;
    input.focus();
    setter.call(input, value);
    input.dispatchEvent(new input.ownerDocument.defaultView.Event('input', { bubbles: true }));
    input.dispatchEvent(new input.ownerDocument.defaultView.Event('change', { bubbles: true }));
  }

  function submit(form) {
    const button = form.querySelector('input[type="submit"], button[type="submit"], input[type="image"], button:not([type])');
    if (button) {
      button.click();
    } else if (typeof form.requestSubmit === 'function') {
      form.requestSubmit();
    } else {
      form.submit();
    }
  }

  // What alert() and confirm() said in any frame (see dialog-hook.js).
  function dialogs(docs) {
    const out = { alerts: [], confirms: [] };
    for (const d of docs) {
      const kept = d.defaultView && d.defaultView.__ebmcpDialogs;
      if (!kept) continue;
      out.alerts.push(...kept.alerts);
      out.confirms.push(...kept.confirms);
    }
    return out;
  }

  // For the diagnosis: each frame's path and what the driver found in it. Never a value typed in.
  function detail(docs) {
    return docs
      .map((d) => {
        let path = '?';
        try {
          path = d.location.pathname;
        } catch {
          path = '?';
        }
        const email = d.getElementById('txtEmail');
        const found = [];
        if (email) found.push(email.form ? 'e-mailveld in formulier' : 'e-mailveld zonder formulier');
        if (d.querySelector('input[type="password"]')) found.push('wachtwoordveld');
        return found.length ? `${path}: ${found.join(', ')}` : path;
      })
      .join(' | ')
      .slice(0, 400);
  }

  function probe() {
    const docs = documents(window);
    return { ...decideProbe(docs), detail: detail(docs) };
  }

  function decideProbe(docs) {
    const said = dialogs(docs);
    if (said.confirms.length) return { phase: 'unknown', message: `e-Boekhouden vraagt: ${clean(said.confirms.at(-1))}` };
    if (window.__ebmcpFilled && said.alerts.length) return { phase: 'error', message: clean(said.alerts.at(-1)) };
    const code = codeInput(docs);
    if (code) return { phase: 'code', message: null };
    const login = loginDocument(docs);
    if (login) {
      const text = clean(login.body?.textContent);
      if (ALREADY.test(text)) return { phase: 'app', message: null };
      if (window.__ebmcpFilled) {
        const error = errorText(login);
        if (error) return { phase: 'error', message: error };
      }
      return { phase: 'form', message: null };
    }
    const anyTwoStep = docs.some((d) => TWO_STEP_TEXT.test(d.body?.textContent ?? ''));
    if (anyTwoStep) return { phase: 'unknown', message: 'e-Boekhouden vraagt om tweestapsverificatie, maar EB MCP vindt het veld niet' };
    if (document.readyState !== 'complete') return { phase: 'loading', message: null };
    if (LOGIN_PATHS.test(location.pathname) && !window.__ebmcpFilled) return { phase: 'loading', message: null };
    return window.__ebmcpFilled ? { phase: 'app', message: null } : { phase: 'loading', message: null };
  }

  function fill(username, password) {
    const login = loginDocument(documents(window));
    if (!login) return 'noform';
    const user = login.getElementById('txtEmail');
    const form = user.form;
    // The password field may sit outside e-Boekhouden's form element; any shown one in the same document will do.
    const pass = form.querySelector('input[type="password"]') ?? [...login.querySelectorAll('input[type="password"]')].find(shown);
    if (!pass) return 'nopassword';
    if (!user.readOnly) setValue(user, username);
    setValue(pass, password);
    window.__ebmcpFilled = true;
    submit(form);
    return 'ok';
  }

  function code(value) {
    const input = codeInput(documents(window));
    if (!input) return 'nofield';
    setValue(input, value);
    window.__ebmcpCode = true;
    if (input.form) submit(input.form);
    return 'ok';
  }

  window.__ebmcpSignin = { probe, fill, code };
})();
