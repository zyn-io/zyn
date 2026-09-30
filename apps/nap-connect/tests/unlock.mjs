// Getting Nap's connect window past its unlock step, whatever this browser
// holds. A fresh browser shows "no wallet here yet" (set up in a tab, or type
// the phrase once behind a link); a browser with a kept wallet asks for its
// password; one already unlocked goes straight to the card.
//
//   await unlockNap(win, words)                 phrase, whatever is shown
//   await unlockNap(win, words, { password })   the password if one is asked

export async function unlockNap(win, words, { password } = {}) {
  await win.waitForSelector('#setup-create, #pw, #yes, #words', { timeout: 30000 });
  if (await win.$('#yes')) return 'card';
  if (password && (await win.$('#pw'))) {
    await win.fill('#pw', password);
    await win.click('#unlock button[type=submit]');
    return 'password';
  }
  if (!(await win.$('#words'))) {
    await win.click('#use-phrase');
    await win.waitForSelector('#words');
  }
  await win.fill('#words', words);
  await win.click('#unlock button[type=submit]');
  return 'phrase';
}
