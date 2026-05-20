class Adder {
	private base Int

	def this(base Int) {
		this.base = base
	}

	def add(value Int) Int {
	    OS.println("value added int " + value)
		this.base + value
	}

	def add(value Str) Int {
	    OS.println("value added string " + value)
    	this.base + 4
    }

	def alterThemAll(items Int...) {
    	for item <- items {
    	    increased = item + 5
    	    OS.println("increased!", increased)
    	    if increased != 9 {
    	        break
    	    }
        }
    }

    def tupleFun() (Int, Str) = (1, "fun")

    def tupleFun2() (amount Int, descr Str) = (2, "fun2")

    def tupleFun3() (amount Int, descr Str) {
        return (2, "fun2")
    }
}

# this is comment!!!

class Bucket {
    base Int := ?

    def this(a Int) {
        this.base = a
    }

    def print() {
        OS.println("base " + this.base)
    }

    def print2() = OS.println("base " + this.base)

    def get2() Int = 5
 }

third := "some string"

def suckItAll(val Str) Str {
    return val + " - hehe"
}

def suckItAll2(val Str) Str {
    val + " - hehe 2"
}

def suckItAll23(val Str) Int {
    23
}

suck23 = suckItAll23("s23-v2")

suck231 () -> Unit = suckItAll23("s23-v3")

suck232 () -> Unit = ()

suck233 () -> Unit = 
	suckItAll23("s23-v3")

suck231()
suck232()
suck233()

def main() Int {
	first Int = 5
	second Int = 7
	third2 = 56
	OS.println("third2 " + third2)

	def x(term Int) = OS.println("xexe" + term)

	def lala() = {
	    OS.println("lala")
	}

	x(5)
	lala()

	counter Int := first
	counter += second

	boost Int = 3
	addBoost = (value Int) -> value + boost

	addBoost2 = value Int -> {
	    value + boost
	}

	OS.println("boost2 " + addBoost2(4))

	lambdaResult Int = addBoost(counter)

	adder = Adder(10)
	adder.alterThemAll(4, 5, 9)
	adder.add(5)
	adder.add("hehe")

	tuple1 = adder.tupleFun()
	t1, t2 := tuple1

	OS.println("tuple1", t1, t2)

	tuple2 = adder.tupleFun2()
    t21, t22 := tuple2
    OS.println("tuple2", t21, t22, tuple2.amount, tuple2.descr)

    OS.println("tuple3", adder.tupleFun3().amount, adder.tupleFun3().descr)

	methodResult Int = adder.add(lambdaResult)

	OS.println("counter " + counter)
	OS.println("lambda " + lambdaResult)
	OS.println("method " + methodResult)

	OS.println("global " + third)

	OS.println(suckItAll("haha"))
	OS.println(suckItAll2("haha 2"))
	OS.println("lalala " + suckItAll23("haha 2"))

	result = if first == 5 {
	    OS.println("YES it's 5!!!")
	    6
	} else {
	    OS.println("NONONO")
	    8
	}

	OS.println("result " + result)

	while true {
	    if counter < 20 {
	        OS.println("counter " + counter)
	    } else {
	        break
	    }
	    counter += 1
	}

	list = [1, 2, 3, 8]

    crapper := 0
	for item <- list {
	    OS.println("item " + item)
	    crapper += item
	}

	OS.println("item end " + crapper)

	newList = for {
	    item <- list
	    item2 <- list
	} yield {
	    item + item2
	}

	for item <- newList {
        OS.println("item", item)
    }

    var1, var2, var3 = 1, crapper, "5a7"

    var4 Int, var5 Float, var6 = 120, 100., "5aa7"

    var7, var8 = 800, adder.add(7) + 700

    var9, var10 := 800, adder.add(7) + 700

    OS.println(var1, var2, var3, var4, var5, var5, var6, var7, var8, var9, var10)

    var9, var10 := 801, adder.add(7) + 701

    OS.println(var1, var2, var3, var4, var5, var5, var6, var7, var8, var9, var10)

    bucket Bucket = Bucket(10)

    bucket.print()

    OS.println("bucket.get2 " + bucket.get2())

	return methodResult
}
