/** Images carry no session cookies or referrer. Only Valve's public CDN hosts. */
export function steamImageUrl(value: string | undefined): string | null {
  if (!value) return null;
  try {
    const url = new URL(value.startsWith("//") ? `https:${value}` : value);
    const host = url.hostname.toLowerCase();
    const allowed = host.endsWith(".steamstatic.com") || host.endsWith(".steamusercontent.com")
      || host === "steamcommunity-a.akamaihd.net" || host === "community.akamai.steamstatic.com"
      || host === "community.cloudflare.steamstatic.com";
    if (!allowed || !["https:", "http:"].includes(url.protocol) || url.username || url.password || url.port) return null;
    url.protocol = "https:";
    return url.href;
  } catch { return null; }
}
