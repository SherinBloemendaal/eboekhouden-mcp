// The sign-in driver against mocked e-Boekhouden pages. Never the real site.
import { beforeEach, describe, expect, it } from 'vitest'
import hookSource from '../../src-tauri/js/dialog-hook.js?raw'
import source from '../../src-tauri/js/signin-driver.js?raw'

interface Driver {
  probe(): { phase: string; message: string | null; detail?: string }
  fill(user: string, pass: string): string
  code(value: string): string
}

const driver = () => (window as unknown as { __ebmcpSignin: Driver }).__ebmcpSignin

/** Top document with one iframe holding the given login markup, like /bh/ > inloggen.asp. */
function page(inner: string): Document {
  document.body.innerHTML = '<iframe name="mainframe"></iframe>'
  const frame = document.querySelector('iframe')!
  const doc = frame.contentDocument!
  doc.open()
  doc.write(`<!doctype html><html><body>${inner}</body></html>`)
  doc.close()
  return doc
}

const FORM = `
  <form id="frm" action="inloggen.asp" method="post">
    <input type="hidden" name="XSRVX" value="abc">
    <input type="hidden" name="FP" value="fp">
    <input id="txtEmail" name="txtEmail" type="text">
    <input id="txtWachtwoord" name="txtWachtwoord" type="password">
    <input type="submit" value="Inloggen">
  </form>`

beforeEach(() => {
  delete (window as unknown as Record<string, unknown>).__ebmcpFilled
  // eslint-disable-next-line no-eval
  window.eval(source)
})

describe('sign-in driver', () => {
  it('finds the form in a nested frame and fills only the visible fields', () => {
    const doc = page(FORM)
    let submitted = 0
    doc.getElementById('frm')!.addEventListener('submit', (e) => {
      submitted += 1
      e.preventDefault()
    })
    expect(driver().probe().phase).toBe('form')
    expect(driver().fill('jan@studionoord.example', 'geheim')).toBe('ok')
    expect((doc.getElementById('txtEmail') as HTMLInputElement).value).toBe('jan@studionoord.example')
    expect((doc.getElementById('txtWachtwoord') as HTMLInputElement).value).toBe('geheim')
    expect((doc.querySelector('[name=XSRVX]') as HTMLInputElement).value).toBe('abc')
    expect(submitted).toBe(1)
  })

  it('reports e-Boekhouden\'s own error after a submit, not before', () => {
    page(`${FORM}<div class="foutmelding">Gebruikersnaam of wachtwoord onjuist.</div>`)
    expect(driver().probe().phase).toBe('form')
    ;(window as unknown as Record<string, unknown>).__ebmcpFilled = true
    expect(driver().probe()).toMatchObject({ phase: 'error', message: 'Gebruikersnaam of wachtwoord onjuist.' })
  })

  it('asks for the two-step code when a code field shows', () => {
    const doc = page(`<form id="f2"><label>Code uit je authenticator-app</label><input id="txtCode" autocomplete="one-time-code"><button>Bevestigen</button></form>`)
    let submitted = 0
    doc.getElementById('f2')!.addEventListener('submit', (e) => {
      submitted += 1
      e.preventDefault()
    })
    expect(driver().probe().phase).toBe('code')
    expect(driver().code('482913')).toBe('ok')
    expect((doc.getElementById('txtCode') as HTMLInputElement).value).toBe('482913')
    expect(submitted).toBe(1)
  })

  it('ignores hidden code-like fields', () => {
    page(`${FORM}<input id="tokenField" type="text" style="display:none">`)
    expect(driver().probe().phase).toBe('form')
  })

  it('stops on a two-step page it does not understand', () => {
    page('<p>Vul je verificatiecode in via de app.</p>')
    expect(driver().probe().phase).toBe('unknown')
  })

  it('treats "reeds ingelogd" as signed in', () => {
    page(`${FORM}<p>U bent reeds ingelogd.</p>`)
    expect(driver().probe().phase).toBe('app')
  })

  it('reports no form instead of guessing', () => {
    page('<p>Even geduld</p>')
    expect(driver().fill('a', 'b')).toBe('noform')
    expect(driver().code('1')).toBe('nofield')
  })

  it('fills a password field that sits outside e-Boekhouden\'s form', () => {
    const doc = page(`<form id="frm"><input id="txtEmail" type="text"><input type="submit" value="Inloggen"></form><input id="pw" type="password">`)
    expect(driver().fill('jan@studionoord.example', 'geheim')).toBe('ok')
    expect((doc.getElementById('pw') as HTMLInputElement).value).toBe('geheim')
  })

  it('names a missing password field instead of trying forever', () => {
    page(`<form id="frm"><input id="txtEmail" type="text"><input type="submit" value="Volgende"></form>`)
    expect(driver().fill('jan@studionoord.example', 'geheim')).toBe('nopassword')
  })

  it('tells the diagnosis what it found in each frame, never a value', () => {
    const doc = page(FORM)
    ;(doc.getElementById('txtEmail') as HTMLInputElement).value = 'jan@studionoord.example'
    const detail = driver().probe().detail ?? ''
    expect(detail).toContain('e-mailveld in formulier, wachtwoordveld')
    expect(detail).not.toContain('jan@studionoord.example')
  })
})

describe('dialogs e-Boekhouden shows', () => {
  function hook(win: Window) {
    // eslint-disable-next-line no-eval
    ;(win as unknown as { eval(code: string): void }).eval(hookSource)
  }

  it('reports an alert after our submit as the error', () => {
    const doc = page(FORM)
    hook(doc.defaultView!)
    doc.defaultView!.alert('Gebruikersnaam en/of wachtwoord onjuist.')
    expect(driver().probe().phase).toBe('form')
    ;(window as unknown as Record<string, unknown>).__ebmcpFilled = true
    expect(driver().probe()).toMatchObject({ phase: 'error', message: 'Gebruikersnaam en/of wachtwoord onjuist.' })
  })

  it('answers confirm with no and stops', () => {
    const doc = page(FORM)
    hook(doc.defaultView!)
    expect(doc.defaultView!.confirm('Je bent elders ingelogd. Doorgaan?')).toBe(false)
    expect(driver().probe()).toMatchObject({ phase: 'unknown', message: 'e-Boekhouden vraagt: Je bent elders ingelogd. Doorgaan?' })
  })
})
