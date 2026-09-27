@app.route(
    "/health",

    methods=["GET"],
)
def blank_decorator():
    return {}
