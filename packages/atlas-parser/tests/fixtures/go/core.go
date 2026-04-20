package sample

import (
    "fmt"
    alias "strings"
    "testing"
)

type Greeter struct{}

func helper(name string) string {
    return alias.ToUpper(name)
}

func caller(name string) string {
    return helper(name)
}

func (g *Greeter) Greet(name string) string {
    return caller(name)
}

func TestGreet(t *testing.T) {
    fmt.Println(helper("atlas"))
}
