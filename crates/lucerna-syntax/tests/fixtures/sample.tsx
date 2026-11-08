import React from "react";

type Props = { name: string };

export function Greeting({ name }: Props) {
  return <div className="greeting">Hello, {name}!</div>;
}
