# EXPECT:
# combo 11
# combo 21
# combo 12
# combo 22
# size 4
# 0

def main() Int {
	items = for {
		left <- [1, 2]
		right <- [10, 20]
	} yield {
		left + right
	}

	for item <- items {
		OS.println("combo " + item)
	}

	OS.println("size " + items.size())
	0
}
