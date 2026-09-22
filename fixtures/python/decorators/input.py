class Service:
    @staticmethod
    def parse(text: str) -> int:
        return int(text)

    @property
    def name(self) -> str:
        return "service"

    @overload
    def handle(self, request: Request) -> Response: ...

    @app.route("/health")
    async def health(self) -> dict[str, str]:
        ...
