import type { CSSProperties } from 'react'

import { layoutDiagram, type QaSlide as Slide } from '@/lib/qa'
import { cn } from '@/lib/utils'

const BOX_W = 200
const BOX_H = 72

/** Type that scales with the slide's width (a container), within readable bounds. */
const fluid = (min: number, cqw: number, max: number): CSSProperties => ({ fontSize: `clamp(${min}px, ${cqw}cqw, ${max}px)` })

/** One presenter slide: its title, points, a code snippet and/or a boxes-and-arrows diagram. */
export function QaSlide({ slide, className }: { slide: Slide | null; className?: string }) {
  if (!slide) {
    return <div className={cn('grid min-h-56 place-items-center rounded-2xl bg-card px-6 text-center text-sm text-dimmer md:aspect-video', className)}>The first slide goes up when the presenter starts</div>
  }
  const visual = slide.code || slide.diagram
  return (
    <div className={cn('@container min-h-56 rounded-2xl bg-card p-[5cqw] md:aspect-video', className)} style={{ containerType: 'inline-size' }}>
      <div className={cn('flex h-full flex-col justify-center gap-[3cqw]', visual && slide.points.length ? 'md:grid md:grid-cols-[minmax(0,5fr)_minmax(0,7fr)] md:content-center md:items-center' : '')}>
        <div className="flex min-w-0 flex-col gap-[2cqw]">
          <h2 className="font-bold tracking-tight text-balance text-foreground" style={{ ...fluid(21, 4.4, 48), lineHeight: 1.1 }}>
            {slide.title}
          </h2>
          {slide.points.length ? (
            <ul className="flex flex-col gap-[1.4cqw] text-muted-foreground" style={{ ...fluid(15, 2.35, 24), lineHeight: 1.4 }}>
              {slide.points.map((p, i) => (
                <li key={i} className="flex gap-2">
                  <span className="mt-[0.55em] size-1.5 shrink-0 rounded-full bg-primary" aria-hidden="true" />
                  <span className="min-w-0">{p}</span>
                </li>
              ))}
            </ul>
          ) : null}
        </div>
        {visual ? (
          <div className="flex min-w-0 flex-col gap-[2cqw]">
            {slide.diagram ? <Diagram diagram={slide.diagram} /> : null}
            {slide.code ? <Code code={slide.code} /> : null}
          </div>
        ) : null}
      </div>
    </div>
  )
}

function Code({ code }: { code: NonNullable<Slide['code']> }) {
  const lines = code.text.split('\n')
  const hl = new Set(code.highlight)
  const width = String(code.start + lines.length - 1).length
  return (
    <figure className="min-w-0 overflow-hidden rounded-xl bg-terminal ring-1 ring-foreground/10">
      {code.path ? (
        <figcaption className="flex border-b border-foreground/10">
          <span className="truncate border-r border-foreground/10 bg-foreground/5 px-3 py-1.5 font-mono text-[11px] text-muted-foreground">{code.path}</span>
        </figcaption>
      ) : null}
      <pre className="overflow-x-auto py-2.5 font-mono" style={{ ...fluid(11, 1.45, 15), lineHeight: 1.6 }}>
        {lines.map((l, i) => (
          <div key={i} className={cn('flex border-l-2 pr-3 pl-2.5', hl.has(i + 1) ? 'border-primary bg-primary/12' : 'border-transparent')}>
            <span className="mr-3 shrink-0 text-right text-dimmer tabular-nums select-none" style={{ width: `${width}ch` }}>
              {code.start + i}
            </span>
            <span className="min-w-0 whitespace-pre-wrap text-foreground [overflow-wrap:anywhere]">{l || ' '}</span>
          </div>
        ))}
      </pre>
    </figure>
  )
}

function Diagram({ diagram }: { diagram: NonNullable<Slide['diagram']> }) {
  const gap = diagram.edges.some((e) => e[2]) ? 160 : 80
  const l = layoutDiagram(diagram, { colW: BOX_W + gap, rowH: BOX_H + 36 })
  const pad = 8
  const w = l.width - gap + pad * 2
  const h = l.height - 36 + pad * 2
  const at = (id: string) => l.pos.get(id)!
  return (
    <svg viewBox={`${-pad} ${-pad} ${w} ${h}`} className="h-auto w-full max-w-full" role="img" aria-label={diagram.nodes.map((n) => n.label).join(', ')}>
      <defs>
        <marker id="qa-arrow" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto-start-reverse">
          <path d="M0,0 L10,5 L0,10 z" className="fill-muted-foreground" />
        </marker>
      </defs>
      {diagram.edges.map(([a, b, label], i) => {
        const p = at(a)
        const q = at(b)
        const forward = q.x > p.x
        const x1 = forward ? p.x + BOX_W : p.x + BOX_W / 2
        const y1 = forward ? p.y + BOX_H / 2 : p.y + (q.y > p.y ? BOX_H : 0)
        const x2 = forward ? q.x : q.x + BOX_W / 2
        const y2 = forward ? q.y + BOX_H / 2 : q.y + (q.y > p.y ? 0 : BOX_H)
        const mx = (x1 + x2) / 2
        const d = forward ? `M${x1},${y1} C${mx},${y1} ${mx},${y2} ${x2},${y2}` : `M${x1},${y1} L${x2},${y2}`
        return (
          <g key={i}>
            <path d={d} fill="none" strokeWidth={2} className="stroke-muted-foreground/70" markerEnd="url(#qa-arrow)" />
            {label ? (
              <text x={mx} y={(y1 + y2) / 2 - 6} textAnchor="middle" className="fill-primary font-mono" style={{ fontSize: 13 }}>
                {label}
              </text>
            ) : null}
          </g>
        )
      })}
      {l.nodes.map((n) => (
        <g key={n.id}>
          <rect x={n.x} y={n.y} width={BOX_W} height={BOX_H} rx={12} strokeWidth={2} className="fill-background stroke-primary/60" />
          <text x={n.x + BOX_W / 2} y={n.y + (n.note ? 30 : 42)} textAnchor="middle" className="fill-foreground font-semibold" style={{ fontSize: 17 }}>
            {n.label}
          </text>
          {n.note ? (
            <text x={n.x + BOX_W / 2} y={n.y + 52} textAnchor="middle" className="fill-muted-foreground" style={{ fontSize: 13 }}>
              {n.note}
            </text>
          ) : null}
        </g>
      ))}
    </svg>
  )
}
