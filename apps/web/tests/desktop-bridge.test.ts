import { describe, expect, it } from 'vitest';
import { incompatibility, normalizeServer, type ServerCheck } from '../src/lib/desktop/bridge';

describe('normalizeServer', () => {
  it('adds https to public hosts and http to local ones', () => {
    expect(normalizeServer('pingo.example.com')).toBe('https://pingo.example.com');
    expect(normalizeServer(' localhost:8090/ ')).toBe('http://localhost:8090');
    expect(normalizeServer('192.168.1.20:8090')).toBe('http://192.168.1.20:8090');
    expect(normalizeServer('172.20.0.2')).toBe('http://172.20.0.2');
    expect(normalizeServer('172.40.0.2')).toBe('https://172.40.0.2');
  });
  it('keeps an explicit scheme', () => {
    expect(normalizeServer('http://pingo.example.com/')).toBe('http://pingo.example.com');
  });
});

describe('incompatibility', () => {
  const check = (compatibility: ServerCheck['compatibility']): ServerCheck => ({
    origin: 'https://pingo.example.com',
    server: { version: '0.4.0', protocol: 2, minProtocol: 2 },
    app: { version: '0.1.0', protocol: 1, minProtocol: 1 },
    compatibility,
  });
  it('is silent when the app fits the server', () => {
    expect(incompatibility(check('compatible'))).toBeNull();
  });
  it('says which side has to update', () => {
    expect(incompatibility(check('clientTooOld'))).toMatch(
      /pingo\.example\.com runs a newer Pingo \(0\.4\.0\).*Update Pingo/,
    );
    expect(incompatibility(check('serverTooOld'))).toMatch(/admin to update it/);
  });
});
