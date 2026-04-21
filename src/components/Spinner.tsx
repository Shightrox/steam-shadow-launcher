interface Props {
  size?: "xs" | "sm" | "md";
  inline?: boolean;
}

const FRAMES = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

export function Spinner({ size = "sm", inline = false }: Props) {
  return (
    <span
      className={`spinner spinner-${size}${inline ? " spinner-inline" : ""}`}
      role="status"
      aria-label="loading"
    >
      {FRAMES.map((f, i) => (
        <span key={i} className="spinner-frame">
          {f}
        </span>
      ))}
    </span>
  );
}
