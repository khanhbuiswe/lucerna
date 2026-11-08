package main

import "fmt"

type Point struct {
	X, Y int
}

func (p Point) Sum() int {
	return p.X + p.Y
}

func main() {
	p := Point{X: 1, Y: 2}
	fmt.Println(p.Sum())
}
