export const serviceCatalog = Object.freeze({
  org: "embedded-alerts",
  title: "Embedded Alerts",
  tagline: "Edge-native alerting for devices, firmware fleets, field sensors, and embedded teams.",
  capabilities: ['intake', 'events', 'alerts', 'leads', 'status', 'analytics'],
  integrations: ["MQTT bridges", "Cloudflare Queues", "Slack", "Prometheus remote write", "Grafana/Loki", "Webhook sinks"],
});

export function normalizeEmail(email) {
  if (typeof email !== 'string') return null;
  const value = email.trim().toLowerCase();
  return /^[^@\s]+@[^@\s]+\.[^@\s]+$/.test(value) ? value : null;
}

export function validateLead(input) {
  if (!input || typeof input !== 'object') return { ok: false, error: 'lead must be an object' };
  const email = normalizeEmail(input.email);
  if (!email) return { ok: false, error: 'valid email is required' };
  const name = String(input.name || '').trim();
  if (name.length < 2) return { ok: false, error: 'name must be at least two characters' };
  return { ok: true, value: { ...input, email, name } };
}

export function makeEvent(type, payload = {}, meta = {}) {
  if (!/^[a-z][a-z0-9_.-]+$/.test(type)) throw new TypeError('event type must be a namespaced lowercase identifier');
  return { id: meta.id || crypto.randomUUID(), type, payload, product: serviceCatalog.org, occurredAt: meta.occurredAt || new Date().toISOString() };
}

export function classifyPriority(signal) {
  const severity = String(signal?.severity || '').toLowerCase();
  if (['critical', 'p0', 'sev0', 'sev1'].includes(severity)) return 'urgent';
  if (['high', 'p1', 'sev2'].includes(severity)) return 'high';
  if (['medium', 'p2', 'warn', 'warning'].includes(severity)) return 'normal';
  return 'low';
}
