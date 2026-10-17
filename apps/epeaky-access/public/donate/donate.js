// Copy buttons: the address from the button's own data, nothing fetched.
const say = (t) => { document.getElementById('say').textContent = t; };
for (const b of document.querySelectorAll('.copy[data-copy]')) {
  b.onclick = async () => {
    try {
      await navigator.clipboard.writeText(b.dataset.copy);
      b.textContent = 'Copied';
      setTimeout(() => { b.textContent = 'Copy address'; }, 1600);
    } catch {
      // No clipboard here: select the address so it can be copied by hand.
      const r = document.createRange();
      r.selectNodeContents(b.parentElement.querySelector('.addr'));
      getSelection().removeAllRanges();
      getSelection().addRange(r);
      say('Select and copy the address by hand.');
    }
  };
}
