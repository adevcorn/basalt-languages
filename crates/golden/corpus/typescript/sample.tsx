import { useState } from "react";

interface Props {
  name: string;
  count?: number;
}

export function Hello({ name, count = 0 }: Props) {
  const [value, setValue] = useState(count);
  const label = `Hello ${name}`;

  function bump() {
    setValue(value + 1);
  }

  return (
    <button onClick={bump} title={label}>
      {label}: {value}
    </button>
  );
}
