import { expect, test } from 'vitest'

import { keyBytes, openRoute, pushSupport, type PushEnv } from './push'

const env = (over: Partial<PushEnv> = {}): PushEnv => ({
  secure: true,
  serviceWorker: true,
  pushManager: true,
  notification: true,
  standalone: true,
  ios: true,
  ...over,
})

test('pushSupport: needs HTTPS, then the push APIs; an iPhone browser tab is told to install', () => {
  expect(pushSupport(env())).toBe('ok')
  expect(pushSupport(env({ secure: false }))).toBe('insecure')
  expect(pushSupport(env({ pushManager: false, notification: false, standalone: false }))).toBe('install')
  expect(pushSupport(env({ pushManager: false, ios: false, standalone: false }))).toBe('unsupported')
  expect(pushSupport(env({ pushManager: false }))).toBe('unsupported')
})

test('keyBytes decodes a base64url key', () => {
  expect(Array.from(keyBytes('AQID_-8'))).toEqual([1, 2, 3, 255, 239])
  expect(keyBytes('BP4z9KsN6nGRTbVYI_c7VJSPQTBtkgcy27mlmlMoZIIgDll6e3vCYLocInmYWAmS6TlzAC8wEqKK6PBru3jl7A8')).toHaveLength(65)
})

test('openRoute takes same-origin hash routes only', () => {
  const origin = 'https://bearclaw.tail.ts.net'
  expect(openRoute(`${origin}/#/s/mac/abc`, origin)).toBe('/s/mac/abc')
  expect(openRoute('/#/', origin)).toBe('/')
  expect(openRoute('https://evil.example/#/s/mac/abc', origin)).toBeNull()
  expect(openRoute(`${origin}/settings`, origin)).toBeNull()
  expect(openRoute(42, origin)).toBeNull()
})
