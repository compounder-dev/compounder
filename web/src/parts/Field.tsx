import { Icon, type IconName } from './Icon.tsx';

type FieldProps = {
  /** What is asked for, for someone who cannot see the field. */
  label: string;
  value: string;
  onChange: (value: string) => void;
  placeholder?: string;
  type?: 'text' | 'search';
  icon?: IconName;
};

/** One line of typing. The label is always there for a reader, whatever the placeholder says. */
export function Field({ label, value, onChange, placeholder, type = 'text', icon }: FieldProps) {
  return (
    <label className="field">
      <span className="sr-only">{label}</span>
      {icon && <Icon name={icon} />}
      <input className="field-input" type={type} value={value} placeholder={placeholder} autoComplete="off" onChange={(event) => onChange(event.target.value)} />
    </label>
  );
}
