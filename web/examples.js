export const examples = {
  hello: {
    label: 'Hello, browser',
    entry: 'main.lum',
    files: {
      'main.lum': `fn main() {
    println("Hello, browser.")
    println("Lume is running locally in WebAssembly.")

    total = [1, 2, 3, 4].fold(0, (sum, value) => sum + value)
    println("Total:", total)
}
`,
    },
  },
  records: {
    label: 'Shapes & collections',
    entry: 'main.lum',
    files: {
      'main.lum': `shape Quote {
    name Str
    price Decimal
}

fn main() {
    quotes = [Quote("North", 12.50d), Quote("South", 8.25d)]
    for quote <- quotes {
        println(quote.name, quote.price)
    }

    total = quotes.fold(0d, (sum, quote) => sum + quote.price)
    println("Total:", total)
}
`,
    },
  },
  modules: {
    label: 'A two-file project',
    entry: 'main.lum',
    files: {
      'main.lum': `use pricing as prices

fn main() {
    println("Price:", prices.discounted(25.00d))
}
`,
      'pricing.lum': `module pricing

public fn discounted(price Decimal) Decimal =
    price * 0.8d
`,
    },
  },
  json: {
    label: 'JSON & regular expressions',
    entry: 'main.lum',
    files: {
      'main.lum': `use lume/json/{Json}
use lume/regex/{Regex}

fn main() {
    values [Int] = Json.decode[[Int]]("[2, 4, 6]")!
    println("Items:", values.size)
    var total Int = 0
    for value <- values { total += value }
    println("Total:", total)

    word = Regex.compile("[a-z]+")!
    println("Matches:", word.matches("hello"))
}
`,
    },
  },
  limits: {
    label: 'Try the execution limit',
    entry: 'main.lum',
    files: {
      'main.lum': `fn main() {
    defer println("Cleanup still runs on a managed limit error.")
    println("Starting a deliberately infinite loop...")
    while true {}
}
`,
    },
  },
};
