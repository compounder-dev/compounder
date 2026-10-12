type SegmentedProps<T extends string> = {
  /** What is being chosen, for someone who cannot see the control. */
  label: string;
  /** One name for each control on a page, so its choices belong together. */
  name: string;
  items: { value: T; label: string }[];
  value: T;
  onChange: (value: T) => void;
};

/** A choice of one among a few, all in view. The arrow keys move the choice, as in any set of radio buttons. */
export function Segmented<T extends string>({ label, name, items, value, onChange }: SegmentedProps<T>) {
  return (
    <fieldset className="segmented">
      <legend className="sr-only">{label}</legend>
      {items.map((item) => (
        <label key={item.value} className="segment">
          <input type="radio" name={name} value={item.value} checked={item.value === value} onChange={() => onChange(item.value)} />
          <span>{item.label}</span>
        </label>
      ))}
    </fieldset>
  );
}
