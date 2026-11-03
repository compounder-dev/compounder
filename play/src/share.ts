// A PnL card as an image, shared through the phone's share sheet or saved.

export async function shareCard({ title, big, sub, tone }: { title: string; big: string; sub: string; tone: 'up' | 'down' | 'flat' }) {
  const w = 1200;
  const h = 630;
  const canvas = Object.assign(document.createElement('canvas'), { width: w, height: h });
  const g = canvas.getContext('2d')!;
  const colour = { up: '#00ec4b', down: '#ff4444', flat: '#bfbfbf' }[tone];
  g.fillStyle = '#000';
  g.fillRect(0, 0, w, h);
  const glow = g.createRadialGradient(w / 2, -60, 0, w / 2, -60, 700);
  glow.addColorStop(0, tone === 'down' ? 'rgba(255,68,68,0.32)' : 'rgba(0,236,75,0.3)');
  glow.addColorStop(1, 'rgba(0,0,0,0)');
  g.fillStyle = glow;
  g.fillRect(0, 0, w, h);
  const font = (size: number, weight = 600) => `${weight} ${size}px "Inter Variable", Inter, system-ui, sans-serif`;
  g.fillStyle = '#fff';
  g.font = font(40);
  g.fillText('compounder', 72, 104);
  g.fillStyle = '#bfbfbf';
  g.font = font(44, 500);
  g.fillText(title, 72, 260);
  g.fillStyle = colour;
  g.font = font(150, 700);
  g.fillText(big, 64, 410);
  g.fillStyle = '#757575';
  g.font = font(36, 500);
  g.fillText(sub, 72, 540);
  const blob = await new Promise<Blob | null>((done) => canvas.toBlob(done, 'image/png'));
  if (!blob) return;
  const file = new File([blob], 'compounder.png', { type: 'image/png' });
  if (navigator.canShare?.({ files: [file] })) {
    await navigator.share({ files: [file], url: window.location.href }).catch(() => {});
    return;
  }
  const a = Object.assign(document.createElement('a'), { href: URL.createObjectURL(blob), download: 'compounder.png' });
  a.click();
  URL.revokeObjectURL(a.href);
}
