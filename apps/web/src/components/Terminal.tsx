// Shows captured CLI output as text, with the state words coloured the way
// the CLI colours them on a terminal.
const states = /\b(running|failed|reconnecting|starting|error|hint)\b/g;

export function Terminal({ text, label }: { text: string; label: string }) {
  return (
    <pre className="terminal" aria-label={label} tabIndex={0}>
      {text.split("\n").map((line, i) => (
        <span key={i} className={line.startsWith("$ ") ? "terminal-prompt" : undefined}>
          {line.startsWith("$ ") ? line : colour(line)}
          {"\n"}
        </span>
      ))}
    </pre>
  );
}

function colour(line: string) {
  const out: React.ReactNode[] = [];
  let last = 0;
  for (const m of line.matchAll(states)) {
    out.push(line.slice(last, m.index));
    out.push(
      <span key={m.index} className={`t-${m[1]}`}>
        {m[1]}
      </span>,
    );
    last = m.index + m[1].length;
  }
  out.push(line.slice(last));
  return out;
}
