import os


class Greeter:
    """Says hello."""

    def __init__(self, name: str) -> None:
        self.name = name

    def greet(self) -> str:
        if self.name:
            return f"Hello, {self.name}"
        return "Hello"


print(Greeter(os.getcwd()).greet())
