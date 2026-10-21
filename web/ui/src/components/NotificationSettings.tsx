import { BellRingIcon } from 'lucide-react'
import { toast } from 'sonner'

import { Button } from '@/components/ui/button'
import { Switch } from '@/components/ui/switch'
import { usePush } from '@/hooks/usePush'
import { useSettings } from '@/hooks/useSettings'

const row = 'flex min-h-12 items-center justify-between gap-3 py-2'

function Toggle({ label, hint, checked, disabled, onChange }: { label: string; hint: string; checked: boolean; disabled?: boolean; onChange: (v: boolean) => void }) {
  return (
    <div className={row}>
      <div className="min-w-0">
        <div className="text-sm">{label}</div>
        <div className="text-xs text-dimmer">{hint}</div>
      </div>
      <Switch checked={checked} disabled={disabled} onCheckedChange={onChange} aria-label={label} />
    </div>
  )
}

/**
 * Settings → Notifications: push notifications on this device when a session waits on you or
 * finishes. The preferences live on the server, per device (lib/push.mjs).
 */
export function NotificationSettings() {
  const { self } = useSettings()
  const { support, device, busy, error, enable, disable, update, test } = usePush()

  if (support === 'off') return null
  const note = (text: string) => <p className="py-3 text-xs text-dimmer">{text}</p>
  if (support === 'install') return note('On iPhone, notifications need the app on your Home Screen: Share → Add to Home Screen, then open bearclaw from there and come back here.')
  if (support === 'insecure') return note('Notifications need HTTPS: open bearclaw through its https:// address (tailscale serve).')
  if (support === 'unsupported') return note('This browser cannot show push notifications.')

  const on = device?.subscribed === true
  const events = device?.events ?? { waiting: true, done: true }
  return (
    <>
      <Toggle
        label="Notifications on this device"
        hint="When a session waits on you or finishes, even with the app closed"
        checked={on}
        disabled={busy || device === null}
        onChange={(v) => void (v ? enable() : disable())}
      />
      {on ? (
        <>
          <Toggle label="Waiting on you" hint="A question, a permission prompt" checked={events.waiting} disabled={busy} onChange={(v) => void update({ events: { waiting: v } })} />
          <Toggle label="Finished" hint="A session is done with its turn" checked={events.done} disabled={busy} onChange={(v) => void update({ events: { done: v } })} />
          <Toggle
            label="Only when I’m away"
            hint={`Nothing while ${self ?? 'the Mac'} has had keyboard or mouse input in the last 2 minutes`}
            checked={device?.awayOnly !== false}
            disabled={busy}
            onChange={(v) => void update({ awayOnly: v })}
          />
          <div className={row}>
            <div className="min-w-0 text-xs text-dimmer">{device?.lastError ? `Last send failed: ${device.lastError}` : 'Sends one now, whatever the settings above'}</div>
            <Button
              variant="outline"
              size="sm"
              disabled={busy}
              onClick={async () => {
                if (await test()) toast.success('Test notification sent')
              }}
            >
              <BellRingIcon /> Test
            </Button>
          </div>
        </>
      ) : null}
      {error ? <p className="py-2 text-xs text-destructive">{error}</p> : null}
    </>
  )
}
