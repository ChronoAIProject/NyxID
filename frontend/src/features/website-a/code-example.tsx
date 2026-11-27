import { useState } from "react";
import type { ReactNode } from "react";

export function CodeExample({
  labels,
  snippets,
}: {
  labels: string[];
  snippets: ReactNode[];
}) {
  const [selected, setSelected] = useState(0);
  return (
    <div className="code">
      <div className="bar">
        <i />
        <i />
        <i />
        <div className="tabs">
          {labels.map((label, index) => (
            <button
              type="button"
              key={label}
              className={index === selected ? "on" : ""}
              aria-pressed={index === selected}
              onClick={() => setSelected(index)}
            >
              {label}
            </button>
          ))}
        </div>
      </div>
      {snippets.map((snippet, index) => (
        <pre
          data-public-panel="code"
          data-public-label={labels[index]}
          hidden={selected !== index}
          key={index}
        >
          {snippet}
        </pre>
      ))}
    </div>
  );
}
