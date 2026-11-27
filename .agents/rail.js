JSON.stringify([...document.querySelectorAll("tr")]
  .filter((r) => /staging-redis|staging-db|prod-api/.test(r.textContent))
  .map((r) => { const c = r.querySelector("td"); return [r.textContent.slice(0, 14), getComputedStyle(c).borderLeftColor, getComputedStyle(c).borderLeftWidth]; }))
