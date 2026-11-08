interface Shape {
  area(): number;
}

export class Square implements Shape {
  constructor(private readonly side: number) {}

  area(): number {
    return this.side * this.side;
  }
}

const shapes: Shape[] = [new Square(2)];
console.log(shapes.map((s) => s.area()));
