const path = require("path");
const HTMLWebpackPlugin = require("html-webpack-plugin");

const isDev = "WEBPACK_DEV_SERVER" in process.env;

module.exports = {
  mode: isDev ? "development" : "production",
  entry: "./src/index.tsx",
  output: {
    filename: "index.js",
    path: path.resolve(__dirname, "dist"),
  },
  plugins: [new HTMLWebpackPlugin({ title: "Domino the Debugger" })],
  performance: {
    // this is going to exclusively be used on localhost
    hints: false,
  },
  devServer: {
    hot: true,
  },
  module: {
    rules: [
      {
        test: /\.css$/,
        use: ["style-loader", "css-loader"],
      },
      {
        test: /\.tsx?$/,
        exclude: /node_modules/,
        use: "babel-loader",
      },
    ],
  },
};
