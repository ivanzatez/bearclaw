import type { ReactNode } from 'react'

import { TabBar } from '@/components/TabBar'
import { useIsDesktop } from '@/hooks/useMediaQuery'
import { cn } from '@/lib/utils'

/**
 * Sticky, translucent top bar with safe-area padding; content is centred at max-w-3xl. On the
 * phone it starts with the tab row (`tabs={false}`: a screen off the row).
 */
export function ScreenHeader({ children, className, tabs = true }: { children: ReactNode; className?: string; tabs?: boolean }) {
  const desktop = useIsDesktop()
  return (
    <header
      className={cn(
        'sticky top-0 z-20 border-b bg-background/90 pt-safe px-safe backdrop-blur-md backdrop-saturate-150',
        className,
      )}
    >
      <div className="mx-auto w-full max-w-3xl px-3 pt-2.5 pb-2">
        {tabs && !desktop ? <TabBar /> : null}
        {children}
      </div>
    </header>
  )
}
