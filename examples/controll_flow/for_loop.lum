# EXPECT:
# item 1
# item 2
# item 3
# item 4
# item 5
# total 15
# 0

def main() Int {
	var total Int = 0

	for item <- [1, 2, 3] {
		OS.println("item " + item)
		total += item
	}

	for item <- Range(4, 6) {
		OS.println("item " + item)
		total += item
	}

	OS.println("total " + total)
	0
}
