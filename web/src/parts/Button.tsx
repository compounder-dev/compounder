import type { ButtonHTMLAttributes, MouseEvent, ReactNode } from 'react';
import { followLink } from '../route.ts';
import { Icon, type IconName } from './Icon.tsx';

export type ButtonVariant = 'primary' | 'secondary' | 'ghost' | 'danger';

type Look = {
  variant?: ButtonVariant;
  size?: 'md' | 'lg';
  /** An icon before the words. */
  icon?: IconName;
  fullWidth?: boolean;
  children: ReactNode;
};

function className({ variant = 'secondary', size = 'md', fullWidth }: Omit<Look, 'children'>) {
  return ['button', `button-${variant}`, size === 'lg' && 'button-lg', fullWidth && 'button-full'].filter(Boolean).join(' ');
}

type ButtonProps = Look & {
  /** Work is under way. The words stay, a spinner joins them, and a second press does nothing. */
  loading?: boolean;
} & Omit<ButtonHTMLAttributes<HTMLButtonElement>, 'className'>;

export function Button({ variant, size, icon, fullWidth, loading = false, children, onClick, type = 'button', ...rest }: ButtonProps) {
  // A loading button keeps the focus it has, so it is marked busy and deaf instead of disabled.
  const press = (event: MouseEvent<HTMLButtonElement>) => {
    if (loading) event.preventDefault();
    else onClick?.(event);
  };
  return (
    <button {...rest} type={type} className={className({ variant, size, fullWidth })} aria-busy={loading || undefined} aria-disabled={loading || undefined} onClick={press}>
      {loading ? <span className="spinner" aria-hidden="true" /> : icon && <Icon name={icon} />}
      {children}
    </button>
  );
}

/** A link that looks like a button, for an action that is a place to go. */
export function ButtonLink({ variant, size, icon, fullWidth, href, reload = false, children }: Look & { href: string; reload?: boolean }) {
  return (
    <a className={className({ variant, size, fullWidth })} href={href} onClick={reload ? undefined : followLink}>
      {icon && <Icon name={icon} />}
      {children}
    </a>
  );
}

type IconButtonProps = {
  icon: IconName;
  /** What the button does, for people who cannot see the icon. */
  label: string;
  variant?: 'ghost' | 'filled';
} & Omit<ButtonHTMLAttributes<HTMLButtonElement>, 'className' | 'children' | 'aria-label'>;

export function IconButton({ icon, label, variant = 'ghost', type = 'button', ...rest }: IconButtonProps) {
  return (
    <button {...rest} type={type} className={`icon-button icon-button-${variant}`} aria-label={label}>
      <Icon name={icon} />
    </button>
  );
}
