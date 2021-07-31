module.exports = {
  preset: 'ts-jest',
  roots: ["<rootDir>/src/"],
  testEnvironment: 'jsdom',
  // setupFilesAfterEnv: ['./jest.setup.ts', "mock-local-storage"],
  moduleFileExtensions: ['ts', 'json', 'js', 'vue'],
  transform: {
    '.*\\.(vue)$': '<rootDir>/node_modules/vue-jest',
    '^.+\\.ts$': 'ts-jest',
  },
  moduleNameMapper: {
    "@/(.*)": "<rootDir>/src/$1",
  },
};
