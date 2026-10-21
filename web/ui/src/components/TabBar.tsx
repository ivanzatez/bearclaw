import { useEffect, useRef } from 'react'
import { useLocation } from 'wouter'

import { useBearclaw } from '@/hooks/useBearclaw'
import { statusSummary } from '@/lib/sessions'
import { TABS, activeTab } from '@/lib/tabs'
import { cn } from '@/lib/utils'

/**
 * The terminal dashboard's tab row (`Sessions · Claw · PRs · …`) on the phone's top-level screens:
 * one line, scrolls sideways, the open tab in the accent colour. Sessions carries the number of
 * sessions waiting on you.
 */
export function TabBar() {
  const [location] = useLocation()
  const { bearclaw } = useBearclaw()
  const waiting = statusSummary(bearclaw).waiting
  const current = activeTab(location)
  const rowRef = useRef<HTMLElement>(null)

  useEffect(() => {
    rowRef.current?.querySelector('[aria-current="page"]')?.scrollIntoView({ block: 'nearest', inline: 'nearest' })
  }, [current])

  return (
    <nav ref={rowRef} aria-label="Tabs" className="no-scrollbar -mx-3 flex gap-0.5 overflow-x-auto px-3 pb-1.5 font-mono text-[0.875rem] leading-6">
      {TABS.map((t) => (
        <a
          key={t.id}
          href={`#${t.route}`}
          aria-current={t.id === current ? 'page' : undefined}
          className={cn(
            'shrink-0 rounded-sm px-1.5 whitespace-nowrap',
            t.id === current ? 'bg-primary font-semibold text-primary-foreground' : 'text-muted-foreground active:text-foreground',
          )}
        >
          {t.label}
          {t.id === 'sessions' && waiting > 0 ? <span className={cn('ml-1 font-semibold', t.id === current ? '' : 'text-status-waiting')}>{waiting}</span> : null}
        </a>
      ))}
    </nav>
  )
}
