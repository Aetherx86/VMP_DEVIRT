#include <stdio.h>
#include <stdlib.h>
#include <stdint.h>
#include <time.h>

// bubble sort
void sort(int *arr, int size) {
    for (int i = 0; i < size - 1; i++) {
        for (int j = 0; j < size - i - 1; j++) {
            if (arr[j] > arr[j + 1]) {
                // swap elements
                int temp = arr[j];
                arr[j] = arr[j + 1];
                arr[j + 1] = temp;
            }
        }
    }
    printf("done\n");
}

int isSorted(int *arr, int size) {
    for (int i = 1; i < size; i++) {
        if (arr[i - 1] > arr[i]) {
            return 0; // Not sorted
        }
    }
    return 1; // Sorted
}

void generateRandomArray(int *arr, int size) {
    int used[size + 1];
    for (int i = 0; i <= size; i++) used[i] = 0;

    for (int i = 0; i < size; i++) {
        int num;
        do {
            num = rand() % (size + 1);
        } while (used[num]);
        arr[i] = num;
        used[num] = 1;
    }
}

int main() {
    srand(time(NULL));
    
    // int size = (rand() % 41) + 10; // Generate a random size between 10 and 50
    int size = 10;
    int *arr = (int *)malloc(sizeof(int) * (size+1));

    // 0x555555952260
    printf("arr address : %p\n", arr);

    generateRandomArray(arr, size);

    printf("Unsorted array: ");
    for (int i = 0; i < size; i++) {
        printf("%d ", arr[i]);
    }
    printf("\n");

    sort(arr, size);

    printf("Sorted array: ");
    for (int i = 0; i < size; i++) {
        printf("%d ", arr[i]);
    }
    printf("\n");

    if (isSorted(arr, size)) {
        printf("Sorted\n");
    } else {
        printf("Not sorted\n");
    }

    return 0;
}