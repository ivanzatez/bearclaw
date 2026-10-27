import type { ReactNode } from 'react'

import { TabBar } from '@/components/TabBar'
import { useIsDesktop } from '@/hooks/useMediaQuery'
import { cn } from '@/lib/utils'

/**
 * Sticky, translucent top bar with safe-area padding; content is centred at max-w-3xl (`wide`: max-w-6xl). On the
 * phone it starts with the tab row (`tabs={false}`: a screen off the row).
 */
export function ScreenHeader({ children, className, tabs = true, wide = false }: { children: ReactNode; className?: string; tabs?: boolean; wide?: boolean }) {
  const desktop = useIsDesktop()
  return (
    <header
      className={cn(
        'sticky top-0 z-20 border-b bg-background/90 pt-safe px-safe backdrop-blur-md backdrop-saturate-150',
        className,
      )}
    >
      <div className={cn('mx-auto w-full px-3 pt-2.5 pb-2', wide ? 'max-w-6xl' : 'max-w-3xl')}>
        {tabs && !desktop ? <TabBar /> : null}
        {children}
      </div>
    </header>
  )
}
