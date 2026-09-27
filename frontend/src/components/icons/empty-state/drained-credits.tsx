import { useId, type SVGProps } from "react";

/**
 * A single coin left where a stack used to be: the spent credits dissolve as
 * dashed outlines drifting upward. Used by the out-of-credits dialog.
 */
export function DrainedCreditsIcon(props: SVGProps<SVGSVGElement>) {
  const maskId = `drained-credits-${useId().replace(/:/g, "")}`;
  return (
    <svg
      xmlns="http://www.w3.org/2000/svg"
      viewBox="-5 -10 110 135"
      fill="none"
      stroke="currentColor"
      strokeWidth={1.1}
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
      {...props}
    >
      <mask
        id={maskId}
        maskUnits="userSpaceOnUse"
        x="-5"
        y="-10"
        width="110"
        height="135"
      >
        <rect x="-5" y="-10" width="110" height="135" fill="#fff" />
        <ellipse cx="50" cy="78" rx="28.2" ry="16.79" fill="#000" />
      </mask>
      <g strokeWidth="1.3">
        <path d="M23.59 74.76A27 15.59 0 0 1 35.69 64.78" />
        <path d="M40.77 63.35A27 15.59 0 1 1 23.59 74.76" />
        <path d="M77 78V84.5A27 15.59 0 0 1 23 84.5V78" />
        <path d="M29.48 78A20.52 11.85 0 1 1 48.21 89.8" />
        <path d="M42.98 89.13A20.52 11.85 0 0 1 30.18 81.07" />
        <path d="M74.67 86.24V88.94" />
        <path d="M68.76 91.11V93.81" />
        <path d="M60.11 94.35V97.05" />
        <path d="M50 95.49V98.19" />
        <path d="M39.89 94.35V97.05" />
        <path d="M31.24 91.11V93.81" />
        <path d="M25.33 86.24V88.94" />
        <path d="M42 78L50 73.38L58 78L50 82.62Z" />
      </g>
      <g mask={`url(#${maskId})`}>
        <g strokeDasharray="2.2 2.8" opacity="0.8">
          <ellipse cx="51.6" cy="70.3" rx="27" ry="15.59" />
          <path d="M24.6 70.3V76.8M78.6 70.3V76.8" />
        </g>
        <g strokeDasharray="2.2 2.8" opacity="0.58">
          <ellipse cx="48.6" cy="61.2" rx="27" ry="15.59" />
          <path d="M21.6 61.2V67.7M75.6 61.2V67.7" />
        </g>
        <g strokeDasharray="2.2 2.8" opacity="0.38">
          <ellipse cx="53" cy="50.3" rx="27" ry="15.59" />
          <path d="M26 50.3V56.8M80 50.3V56.8" />
        </g>
      </g>
      <circle cx="62" cy="18" r="1.2" fill="currentColor" stroke="none" />
      <circle cx="68" cy="10" r="0.95" fill="currentColor" stroke="none" />
      <circle cx="60" cy="3" r="0.75" fill="currentColor" stroke="none" />
      <path d="M87 23Q88 27 92 28Q88 29 87 33Q86 29 82 28Q86 27 87 23Z" />
      <path d="M13 38.6Q13.68 41.32 16.4 42Q13.68 42.68 13 45.4Q12.32 42.68 9.6 42Q12.32 41.32 13 38.6Z" />
      <path d="M91 47.5Q91.5 49.5 93.5 50Q91.5 50.5 91 52.5Q90.5 50.5 88.5 50Q90.5 49.5 91 47.5Z" />
    </svg>
  );
}
