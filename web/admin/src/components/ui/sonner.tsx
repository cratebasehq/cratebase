import { useTheme } from "next-themes"
import { Toaster as Sonner, type ToasterProps } from "sonner"
import { CircleCheckIcon, InfoIcon, TriangleAlertIcon, OctagonXIcon, Loader2Icon } from "lucide-react"

const Toaster = ({ ...props }: ToasterProps) => {
  const { theme = "system" } = useTheme()

  return (
    <Sonner
      theme={theme as ToasterProps["theme"]}
      className="toaster group"
      icons={{
        success: (
          <CircleCheckIcon className="size-4" />
        ),
        info: (
          <InfoIcon className="size-4" />
        ),
        warning: (
          <TriangleAlertIcon className="size-4" />
        ),
        error: (
          <OctagonXIcon className="size-4" />
        ),
        loading: (
          <Loader2Icon className="size-4 animate-spin" />
        ),
      }}
      style={
        {
          "--normal-bg": "var(--popover)",
          "--normal-text": "var(--popover-foreground)",
          "--normal-border": "var(--border)",
          "--success-bg": "color-mix(in oklab, var(--success) 10%, var(--popover))",
          "--success-text": "var(--popover-foreground)",
          "--success-border": "color-mix(in oklab, var(--success) 35%, var(--border))",
          "--error-bg": "color-mix(in oklab, var(--destructive) 10%, var(--popover))",
          "--error-text": "var(--popover-foreground)",
          "--error-border": "color-mix(in oklab, var(--destructive) 40%, var(--border))",
          "--warning-bg": "color-mix(in oklab, var(--warning) 12%, var(--popover))",
          "--warning-text": "var(--popover-foreground)",
          "--warning-border": "color-mix(in oklab, var(--warning) 40%, var(--border))",
          "--info-bg": "color-mix(in oklab, var(--info) 10%, var(--popover))",
          "--info-text": "var(--popover-foreground)",
          "--info-border": "color-mix(in oklab, var(--info) 35%, var(--border))",
          "--border-radius": "var(--radius)",
        } as React.CSSProperties
      }
      richColors
      closeButton
      duration={4000}
      toastOptions={{
        classNames: {
          toast: "cn-toast font-sans! text-sm! shadow-e3!",
          title: "font-medium!",
          description: "text-muted-foreground!",
          icon: "mt-px",
        },
      }}
      {...props}
    />
  )
}

export { Toaster }
