import { Link } from "@tanstack/react-router";
import { GitHubButton } from "@/components/shared/github-button";

export function LandingFooter() {
  return (
    <footer className="border-t border-border/60 px-6 py-6">
      <div className="mx-auto flex max-w-6xl flex-col items-center gap-3 sm:flex-row sm:justify-between">
        <p className="text-xs text-text-tertiary">
          &copy; {new Date().getFullYear()} Chrono AI. All rights reserved.
        </p>
        <div className="flex items-center gap-4">
          <Link
            to="/terms"
            className="text-xs text-text-tertiary transition-colors hover:text-foreground"
          >
            Terms of Use
          </Link>
          <Link
            to="/privacy"
            className="text-xs text-text-tertiary transition-colors hover:text-foreground"
          >
            Privacy
          </Link>
          <a
            href="https://discord.gg/QMvcs8UQBW"
            target="_blank"
            rel="noopener noreferrer"
            className="text-text-tertiary transition-colors hover:text-foreground"
          >
            <svg width="20" height="20" viewBox="0 0 24 24" fill="currentColor" aria-label="Discord">
              <path d="M20.317 4.37a19.791 19.791 0 0 0-4.885-1.515.074.074 0 0 0-.079.037c-.21.375-.444.864-.608 1.25a18.27 18.27 0 0 0-5.487 0 12.64 12.64 0 0 0-.617-1.25.077.077 0 0 0-.079-.037A19.736 19.736 0 0 0 3.677 4.37a.07.07 0 0 0-.032.027C.533 9.046-.32 13.58.099 18.057a.082.082 0 0 0 .031.057 19.9 19.9 0 0 0 5.993 3.03.078.078 0 0 0 .084-.028c.462-.63.874-1.295 1.226-1.994a.076.076 0 0 0-.041-.106 13.107 13.107 0 0 1-1.872-.892.077.077 0 0 1-.008-.128 10.2 10.2 0 0 0 .372-.292.074.074 0 0 1 .077-.01c3.928 1.793 8.18 1.793 12.062 0a.074.074 0 0 1 .078.01c.12.098.246.198.373.292a.077.077 0 0 1-.006.127 12.299 12.299 0 0 1-1.873.892.077.077 0 0 0-.041.107c.36.698.772 1.362 1.225 1.993a.076.076 0 0 0 .084.028 19.839 19.839 0 0 0 6.002-3.03.077.077 0 0 0 .032-.054c.5-5.177-.838-9.674-3.549-13.66a.061.061 0 0 0-.031-.03zM8.02 15.33c-1.183 0-2.157-1.085-2.157-2.419 0-1.333.956-2.419 2.157-2.419 1.21 0 2.176 1.095 2.157 2.42 0 1.333-.956 2.418-2.157 2.418zm7.975 0c-1.183 0-2.157-1.085-2.157-2.419 0-1.333.956-2.419 2.157-2.419 1.21 0 2.176 1.095 2.157 2.42 0 1.333-.947 2.418-2.157 2.418z" />
            </svg>
          </a>
          <GitHubButton className="text-text-tertiary hover:text-foreground" />
        </div>
      </div>
    </footer>
  );
}
